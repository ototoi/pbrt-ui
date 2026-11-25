//! A texture resource and its upstream are inputs to the evaluator's
//! tree, and the preview PNG `project.preview_texture` caches. Parameters
//! resolve as r4 reads them: the declaration if any, else the table's
//! table's default, with the table taking precedence; images decode here and are linear according to `encoding`.
use project::{ParameterValues, ProjectDocument, Resource, ResourceId, ResourceKind};
use scene_parse::texture_eval::{
    IDENTITY, Image, Mapping2D, Matrix, Operand, Rgb, Texture, Wrap, spectra,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Deeper upstream chains than this are not drawn (the document refuses
/// cycles; this only bounds pathological depth).
pub(super) const MAX_DEPTH: usize = 64;

/// A tree that would expand past this many nodes is not drawn: shared
/// upstream textures are evaluated once per reference, so a DAG that
/// reuses its inputs grows exponentially.
pub(super) const MAX_NODES: usize = 4096;

/// Bumped whenever the evaluator, the defaults it reads or the PNG
/// encoding change, so cached previews from older code miss.
const PREVIEW_VERSION: &str = "texture-eval 3";

/// How an 8-bit image's values become linear (r4's `ColorEncoding`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Encoding {
    Linear,
    Srgb,
    Gamma(f32),
}

/// r4's legacy `gamma` parameter: a value, or a bool meaning sRGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Gamma {
    Value(f32),
    Legacy(bool),
}

/// r4's imagemap encoding rule: an explicit `encoding` (sRGB / srgb /
/// linear / "gamma G"), else the legacy `gamma`, else `.png` sRGB and
/// every other extension linear. Float images (EXR) are read linear
/// whatever is declared.
pub(super) fn encoding_of(
    extension: &str,
    encoding: Option<&str>,
    gamma: Option<Gamma>,
    float: bool,
) -> Result<Encoding, String> {
    if float {
        return Ok(Encoding::Linear);
    }
    if let Some(name) = encoding.filter(|name| !name.is_empty()) {
        return match name {
            "linear" => Ok(Encoding::Linear),
            "sRGB" | "srgb" => Ok(Encoding::Srgb),
            _ => match name.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["gamma", value] => value
                    .parse::<f32>()
                    .map(Encoding::Gamma)
                    .map_err(|_| format!("invalid color encoding \"{name}\"")),
                _ => Err(format!("unknown color encoding \"{name}\"")),
            },
        };
    }
    Ok(match gamma {
        Some(Gamma::Value(value)) => Encoding::Gamma(value),
        Some(Gamma::Legacy(true)) => Encoding::Srgb,
        Some(Gamma::Legacy(false)) => Encoding::Linear,
        None if extension.eq_ignore_ascii_case("png") => Encoding::Srgb,
        None => Encoding::Linear,
    })
}

pub(super) struct Builder<'a> {
    pub(super) document: &'a ProjectDocument,
    pub(super) asset_root: Option<&'a str>,
    pub(super) snapshot: Option<&'a project::ProjectSnapshot>,
    /// False for the key pass: images are hashed, never decoded, so a
    /// cache hit costs no decode.
    decode: bool,
    /// What the preview depends on: each resource's content and each
    /// image's SHA-256 (editions repeat across sessions; content does not).
    pub(super) inputs: Vec<String>,
    /// Decoded images by asset, encoding and lineage.
    images: BTreeMap<String, Arc<Image>>,
    /// Each upstream texture is built once and shared.
    built: BTreeMap<ResourceId, Arc<Texture>>,
    /// Likewise each material.
    pub(super) materials: BTreeMap<ResourceId, Arc<scene_parse::material_preview::Material>>,
}

/// The parameters of one resource, declared value first, then the table.
pub(super) struct Parameters<'a> {
    pub(super) resource: &'a Resource,
    pub(super) rows: &'a [pbrt_io::parameter_schema::ParameterSpec],
}

impl<'a> Parameters<'a> {
    pub(super) fn declared(&self, name: &str) -> Option<project::Parameter> {
        self.resource
            .parameters
            .iter()
            .find(|p| {
                p.name == name
                    || self
                        .rows
                        .iter()
                        .any(|spec| spec.name == name && spec.aliases.contains(&p.name.as_str()))
            })
            .cloned()
            .or_else(|| {
                self.rows
                    .iter()
                    .find(|spec| spec.name == name)
                    .and_then(|spec| spec.default_parameter())
            })
    }

    pub(super) fn floats(&self, name: &str) -> Option<Vec<f32>> {
        match self.declared(name)?.values {
            ParameterValues::Floats(values) => Some(
                values
                    .iter()
                    .map(|v| match v {
                        project::FloatValue::Finite(v) => *v as f32,
                        _ => 0.,
                    })
                    .collect(),
            ),
            ParameterValues::Integers(values) => Some(values.iter().map(|v| *v as f32).collect()),
            _ => None,
        }
    }

    pub(super) fn float(&self, name: &str, fallback: f32) -> f32 {
        self.floats(name)
            .and_then(|values| values.first().copied())
            .unwrap_or(fallback)
    }

    pub(super) fn vector(&self, name: &str, fallback: [f32; 3]) -> [f32; 3] {
        match self.floats(name) {
            Some(values) if values.len() == 3 => [values[0], values[1], values[2]],
            _ => fallback,
        }
    }

    pub(super) fn string(&self, name: &str, fallback: &str) -> String {
        match self.declared(name).map(|p| p.values) {
            Some(ParameterValues::Strings(values)) if !values.is_empty() => values[0].clone(),
            _ => fallback.to_owned(),
        }
    }

    pub(super) fn boolean(&self, name: &str) -> bool {
        matches!(
            self.declared(name).map(|p| p.values),
            Some(ParameterValues::Booleans(values)) if values.first() == Some(&true)
        )
    }

    pub(super) fn texture(&self, name: &str) -> Option<ResourceId> {
        match self.declared(name)?.values {
            ParameterValues::Resources {
                kind: ResourceKind::Texture,
                ids,
            } => ids.first().copied(),
            _ => None,
        }
    }
}

/// The world-to-texture matrix: the inverse of the declaration CTM.
fn texture_from_world(resource: &Resource) -> Matrix {
    let Some(start) = resource
        .declared_in
        .as_ref()
        .and_then(|declared| declared.transform.as_ref())
        .map(|transform| transform.start)
    else {
        return IDENTITY;
    };
    let Some(inverse) = project::math::inverse(project::math::Mat4(start)) else {
        return IDENTITY;
    };
    std::array::from_fn(|row| std::array::from_fn(|col| inverse.0[row * 4 + col] as f32))
}

impl<'a> Builder<'a> {
    pub(super) fn operand(
        &mut self,
        parameters: &Parameters,
        name: &str,
        fallback: Rgb,
        depth: usize,
    ) -> Result<Operand, String> {
        if let Some(id) = parameters.texture(name) {
            return Ok(Operand::Texture(self.texture(id, depth + 1)?));
        }
        let declared = parameters.declared(name);
        let value_type = declared.as_ref().map(|p| p.value_type.as_str());
        Ok(Operand::Constant(
            match (value_type, parameters.floats(name).as_deref()) {
                (Some("float"), Some([value])) => [*value; 3],
                (Some("rgb"), Some([r, g, b])) => [*r, *g, *b],
                // Spectra read at one wavelength per channel (2
                // E): `λ value` pairs, or a builtin name.
                (Some("spectrum"), Some(pairs)) => spectra::sampled(pairs).unwrap_or(fallback),
                (Some("spectrum"), None) => match declared.map(|p| p.values) {
                    Some(
                        ParameterValues::BuiltinSpectra(names) | ParameterValues::Strings(names),
                    ) => names
                        .first()
                        .and_then(|name| spectra::named(name))
                        .unwrap_or(fallback),
                    _ => fallback,
                },
                // Blackbody temperatures are not RGB: drawn with the
                // neutral fallback.
                _ => fallback,
            },
        ))
    }

    fn mapping(&self, parameters: &Parameters, resource: &Resource) -> Mapping2D {
        match parameters.string("mapping", "uv").as_str() {
            "spherical" => Mapping2D::Spherical(texture_from_world(resource)),
            "cylindrical" => Mapping2D::Cylindrical(texture_from_world(resource)),
            "planar" => Mapping2D::Planar {
                texture_from_world: texture_from_world(resource),
                vs: parameters.vector("v1", [1., 0., 0.]),
                vt: parameters.vector("v2", [0., 1., 0.]),
                ds: parameters.float("udelta", 0.),
                dt: parameters.float("vdelta", 0.),
            },
            _ => Mapping2D::Uv {
                su: parameters.float("uscale", 1.),
                sv: parameters.float("vscale", 1.),
                du: parameters.float("udelta", 0.),
                dv: parameters.float("vdelta", 0.),
            },
        }
    }

    /// The image an imagemap reads, decoded linear. A float texture reads
    /// one value per texel as r4's float MIP-map does: the single channel,
    /// the RGB average, or the alpha of an RGBA image.
    fn image(
        &mut self,
        parameters: &Parameters,
        float_texture: bool,
    ) -> Result<Arc<Image>, String> {
        let asset_id = match parameters.declared("filename").map(|p| p.values) {
            Some(ParameterValues::Assets(ids)) if !ids.is_empty() => ids[0],
            _ => return Err("imagemap has no image".into()),
        };
        let asset = self
            .document
            .assets
            .get(&asset_id)
            .ok_or("imagemap image is missing")?;
        self.inputs.push(format!("image {}", asset.sha256));
        let gamma = match parameters.declared("gamma").map(|p| p.values) {
            Some(ParameterValues::Floats(values)) => values.first().map(|v| match v {
                project::FloatValue::Finite(v) => Gamma::Value(*v as f32),
                _ => Gamma::Value(1.),
            }),
            Some(ParameterValues::Booleans(values)) => values.first().map(|v| Gamma::Legacy(*v)),
            _ => None,
        };
        let declared = parameters
            .declared("encoding")
            .and_then(|p| match p.values {
                ParameterValues::Strings(values) => values.first().cloned(),
                _ => None,
            });
        // One asset decodes differently per encoding and per lineage.
        let identity = format!("{} {declared:?} {gamma:?} {float_texture}", asset_id.0);
        if let Some(image) = self.images.get(&identity) {
            return Ok(image.clone());
        }
        if !self.decode {
            return Ok(Arc::new(Image {
                width: 0,
                height: 0,
                pixels: Vec::new(),
            }));
        }
        let bytes = verified_asset_bytes(self.snapshot, self.asset_root, asset)?;
        // Radiance HDR keeps its float light (image 0.24's DynamicImage
        // path would clamp it to gamma-encoded Rgb8).
        let decoded = if scene_parse::hdr::is_hdr(&bytes, &asset.location) {
            scene_parse::hdr::decode(&bytes)
                .map_err(|error| format!("cannot decode {}: {error}", asset.location))?
        } else {
            let mut reader = image::io::Reader::new(std::io::Cursor::new(bytes));
            reader = reader
                .with_guessed_format()
                .map_err(|error| format!("cannot read {}: {error}", asset.location))?;
            if reader.format().is_none() {
                reader.set_format(
                    image::ImageFormat::from_path(&asset.location)
                        .map_err(|error| format!("unknown image format: {error}"))?,
                );
            }
            let mut limits = image::io::Limits::default();
            limits.max_image_width = Some(16_384);
            limits.max_image_height = Some(16_384);
            limits.max_alloc = Some(1024 * 1024 * 1024);
            reader.limits(limits);
            reader
                .decode()
                .map_err(|error| format!("cannot decode {}: {error}", asset.location))?
        };
        let float = matches!(
            decoded,
            image::DynamicImage::ImageRgb32F(_) | image::DynamicImage::ImageRgba32F(_)
        );
        let extension = std::path::Path::new(&asset.location)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_owned();
        let encoding = encoding_of(&extension, declared.as_deref(), gamma, float)?;
        let decode = |v: f32| -> f32 {
            match encoding {
                Encoding::Linear => v,
                Encoding::Srgb => {
                    if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                }
                Encoding::Gamma(gamma) => v.powf(gamma),
            }
        };
        let channels = decoded.color().channel_count();
        let rgba = decoded.to_rgba32f();
        let pixels = rgba
            .pixels()
            .map(|pixel| {
                let rgb = [decode(pixel[0]), decode(pixel[1]), decode(pixel[2])];
                if !float_texture {
                    return rgb;
                }
                [match channels {
                    1 | 2 => rgb[0],
                    4 => pixel[3],
                    _ => (rgb[0] + rgb[1] + rgb[2]) / 3.,
                }; 3]
            })
            .collect();
        let image = Arc::new(Image {
            width: rgba.width() as usize,
            height: rgba.height() as usize,
            pixels,
        });
        self.images.insert(identity, image.clone());
        Ok(image)
    }

    pub(super) fn texture(&mut self, id: ResourceId, depth: usize) -> Result<Arc<Texture>, String> {
        if depth > MAX_DEPTH {
            return Err("texture chain is too deep".into());
        }
        if let Some(texture) = self.built.get(&id) {
            return Ok(texture.clone());
        }
        let resource = self
            .document
            .resources
            .get(&id)
            .filter(|resource| resource.kind == ResourceKind::Texture)
            .ok_or("not a texture")?;
        self.inputs.push(format!(
            "texture {} {}",
            id.0,
            serde_json::json!({
                "implementation": resource.implementation,
                "texture_value_type": resource.texture_value_type,
                "parameters": resource.parameters,
                "declared_in": resource.declared_in,
            })
        ));
        let texture = Arc::new(self.build_texture(resource, depth)?);
        self.built.insert(id, texture.clone());
        Ok(texture)
    }

    fn build_texture(&mut self, resource: &Resource, depth: usize) -> Result<Texture, String> {
        let rows = pbrt_io::parameter_schema::schema("Texture")
            .and_then(|table| {
                pbrt_io::parameter_schema::implementation(table, &resource.implementation)
            })
            .and_then(|found| found.parameters(resource.texture_value_type))
            .unwrap_or(&[]);
        let parameters = Parameters { resource, rows };
        let p = &parameters;
        let (zero, one) = ([0.; 3], [1.; 3]);
        Ok(match resource.implementation.as_str() {
            "constant" => match self.operand(p, "value", one, depth)? {
                Operand::Constant(value) => Texture::Constant(value),
                Operand::Texture(texture) => (*texture).clone(),
            },
            "scale" => Texture::Scale {
                tex: self.operand(p, "tex", one, depth)?,
                scale: self.operand(p, "scale", one, depth)?,
            },
            "mix" => Texture::Mix {
                tex1: self.operand(p, "tex1", zero, depth)?,
                tex2: self.operand(p, "tex2", one, depth)?,
                amount: self.operand(p, "amount", [0.5; 3], depth)?,
            },
            "directionmix" => {
                // r4 takes dir through the declaration CTM: world space.
                let dir = p.vector("dir", [0., 1., 0.]);
                let m = resource
                    .declared_in
                    .as_ref()
                    .and_then(|declared| declared.transform.as_ref())
                    .map(|transform| transform.start);
                let dir = match m {
                    Some(m) => {
                        let row = |r: usize| {
                            (m[r * 4] * dir[0] as f64
                                + m[r * 4 + 1] * dir[1] as f64
                                + m[r * 4 + 2] * dir[2] as f64) as f32
                        };
                        [row(0), row(1), row(2)]
                    }
                    None => dir,
                };
                let length = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
                let dir = if length > 0. {
                    dir.map(|v| v / length)
                } else {
                    dir
                };
                Texture::DirectionMix {
                    tex1: self.operand(p, "tex1", zero, depth)?,
                    tex2: self.operand(p, "tex2", one, depth)?,
                    dir,
                }
            }
            "checkerboard" => {
                let tex1 = self.operand(p, "tex1", one, depth)?;
                let tex2 = self.operand(p, "tex2", zero, depth)?;
                if p.float("dimension", 2.) == 3. {
                    Texture::Checkerboard3D {
                        texture_from_world: texture_from_world(resource),
                        tex1,
                        tex2,
                    }
                } else {
                    Texture::Checkerboard2D {
                        mapping: self.mapping(p, resource),
                        tex1,
                        tex2,
                    }
                }
            }
            // As in pbrt-v4 and r4, `inside` shows outside the dots and
            // `outside` within them, the reverse of what the names suggest.
            "dots" => Texture::Dots {
                mapping: self.mapping(p, resource),
                outside: self.operand(p, "inside", one, depth)?,
                inside: self.operand(p, "outside", zero, depth)?,
            },
            "fbm" => Texture::Fbm {
                texture_from_world: texture_from_world(resource),
                octaves: p.float("octaves", 8.) as u32,
                omega: p.float("roughness", 0.5),
            },
            "wrinkled" => Texture::Wrinkled {
                texture_from_world: texture_from_world(resource),
                octaves: p.float("octaves", 8.) as u32,
                omega: p.float("roughness", 0.5),
            },
            "windy" => Texture::Windy {
                texture_from_world: texture_from_world(resource),
            },
            "marble" => Texture::Marble {
                texture_from_world: texture_from_world(resource),
                octaves: p.float("octaves", 8.) as u32,
                omega: p.float("roughness", 0.5),
                scale: p.float("scale", 1.),
                variation: p.float("variation", 0.2),
            },
            "bilerp" => {
                let value = |builder: &mut Self, name, fallback| {
                    builder
                        .operand(p, name, fallback, depth)
                        .map(|operand| match operand {
                            Operand::Constant(value) => value,
                            Operand::Texture(_) => fallback,
                        })
                };
                Texture::Bilerp {
                    mapping: self.mapping(p, resource),
                    v00: value(self, "v00", zero)?,
                    v01: value(self, "v01", one)?,
                    v10: value(self, "v10", zero)?,
                    v11: value(self, "v11", one)?,
                }
            }
            "imagemap" => Texture::Image {
                mapping: self.mapping(p, resource),
                image: self.image(
                    p,
                    resource.texture_value_type == Some(project::TextureValueType::Float),
                )?,
                wrap: match p.string("wrap", "repeat").as_str() {
                    "black" => Wrap::Black,
                    "clamp" => Wrap::Clamp,
                    _ => Wrap::Repeat,
                },
                scale: p.float("scale", 1.),
                invert: p.boolean("invert"),
            },
            other => return Err(format!("no preview for {other} textures")),
        })
    }
}

/// Whether `texture` expands to at most `budget` nodes when evaluated
/// (shared inputs count once per reference).
pub(super) fn expanded(texture: &Texture, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    let operands: Vec<&Operand> = match texture {
        Texture::Scale { tex, scale } => vec![tex, scale],
        Texture::Mix { tex1, tex2, amount } => vec![tex1, tex2, amount],
        Texture::DirectionMix { tex1, tex2, .. }
        | Texture::Checkerboard2D { tex1, tex2, .. }
        | Texture::Checkerboard3D { tex1, tex2, .. } => vec![tex1, tex2],
        Texture::Dots {
            inside, outside, ..
        } => vec![inside, outside],
        _ => Vec::new(),
    };
    operands.into_iter().all(|operand| match operand {
        Operand::Texture(texture) => expanded(texture, budget),
        Operand::Constant(_) => true,
    })
}

pub(super) fn builder<'a>(
    document: &'a ProjectDocument,
    asset_root: Option<&'a str>,
    decode: bool,
) -> Builder<'a> {
    Builder {
        document,
        asset_root,
        snapshot: None,
        decode,
        inputs: Vec::new(),
        images: BTreeMap::new(),
        built: BTreeMap::new(),
        materials: BTreeMap::new(),
    }
}

/// The asset's bytes, read once and verified against its immutable record:
/// leased staging bytes first (Viewer), then the model-placed file. This
/// keeps the bytes used to build a key aligned with those decoded on a
/// cache miss, even if the backing file changes while a preview runs.
pub(super) fn verified_asset_bytes(
    snapshot: Option<&project::ProjectSnapshot>,
    asset_root: Option<&str>,
    asset: &project::Asset,
) -> Result<Vec<u8>, String> {
    let path = if let Some(snapshot) = snapshot {
        if let Some(lease) = snapshot.payload_by(asset.id) {
            lease.path().to_path_buf()
        } else {
            let root = asset_root.ok_or("the project has no asset folder")?;
            snapshot
                .folder_asset_path(std::path::Path::new(root), asset.id)
                .map_err(|error| error.to_string())?
        }
    } else {
        let root = asset_root.ok_or("the project has no asset folder")?;
        std::path::Path::new(root).join(&asset.location)
    };
    let bytes =
        std::fs::read(&path).map_err(|error| format!("cannot read {}: {error}", asset.location))?;
    if bytes.len() as u64 != asset.size || format!("{:x}", Sha256::digest(&bytes)) != asset.sha256 {
        return Err(format!(
            "image asset {} changed or failed validation",
            asset.location
        ));
    }
    Ok(bytes)
}

/// The cache key of `texture`'s preview at `size`, without decoding any
/// image: it changes whenever the texture, an upstream texture, an image
/// it reads or the evaluator version changes.
pub(super) fn key(
    document: &ProjectDocument,
    texture: ResourceId,
    asset_root: Option<&str>,
    size: u32,
) -> Result<String, String> {
    let mut builder = builder(document, asset_root, false);
    let tree = builder.texture(texture, 0)?;
    if !expanded(&tree, &mut MAX_NODES.clone()) {
        return Err("the texture graph is too large to preview".into());
    }
    Ok(cache_key("texture", PREVIEW_VERSION, size, &builder.inputs))
}

/// `<prefix>-<hash>.png` over the version, the size and every input.
pub(super) fn cache_key(prefix: &str, version: &str, size: u32, inputs: &[String]) -> String {
    let mut hash = Sha256::new();
    hash.update(format!("{version}\nsize {size}\n"));
    for input in inputs {
        hash.update(input.as_bytes());
        hash.update(b"\n");
    }
    let digest = hash.finalize();
    format!(
        "{prefix}-{}.png",
        digest[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

/// The evaluator's tree for `texture`, images decoded (a cache miss).
#[cfg(test)]
pub(super) fn tree(
    document: &ProjectDocument,
    texture: ResourceId,
    asset_root: Option<&str>,
) -> Result<Arc<Texture>, String> {
    builder(document, asset_root, true).texture(texture, 0)
}

pub(super) fn tree_from_snapshot<'a>(
    snapshot: &'a project::ProjectSnapshot,
    texture: ResourceId,
    asset_root: Option<&'a str>,
) -> Result<Arc<Texture>, String> {
    let mut builder = builder(snapshot.document(), asset_root, true);
    builder.snapshot = Some(snapshot);
    builder.texture(texture, 0)
}

/// Both, for tests: the tree (images decoded) and its key.
#[cfg(test)]
fn build(
    document: &ProjectDocument,
    texture: ResourceId,
    asset_root: Option<&str>,
    size: u32,
) -> Result<(Texture, String), String> {
    let key = key(document, texture, asset_root, size)?;
    Ok(((*tree(document, texture, asset_root)?).clone(), key))
}

/// The preview as PNG bytes.
pub(super) fn png(tree: &Texture, size: u32) -> Result<Vec<u8>, String> {
    encode_png(scene_parse::texture_eval::thumbnail(tree, size), size)
}

/// RGBA8 rows as PNG bytes.
pub(super) fn encode_png(rgba: Vec<u8>, size: u32) -> Result<Vec<u8>, String> {
    let image = image::RgbaImage::from_raw(size, size, rgba).ok_or("bad preview buffer")?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageOutputFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::{ColorSpaceBinding, FloatValue, Parameter, Revision, TextureValueType};

    fn texture(
        document: &mut ProjectDocument,
        implementation: &str,
        parameters: Vec<Parameter>,
    ) -> ResourceId {
        let resource = Resource {
            id: ResourceId::new(),
            label: implementation.into(),
            location: format!("Textures/{implementation}.texture"),
            edition: Revision::initial(),
            kind: ResourceKind::Texture,
            texture_value_type: Some(TextureValueType::Float),
            implementation: implementation.into(),
            parameters,
            definition_scope: None,
            declared_in: None,
            origin: None,
        };
        let id = resource.id;
        document.resources.insert(id, resource);
        id
    }

    fn float(name: &str, value: f64) -> Parameter {
        Parameter {
            name: name.into(),
            value_type: "float".into(),
            color_space: ColorSpaceBinding::Inherit,
            values: ParameterValues::Floats(vec![FloatValue::Finite(value)]),
        }
    }

    #[test]
    fn the_key_follows_content_not_editions() {
        // Editions repeat across sessions (discarding unsaved edits rewinds
        // them), but the cache persists: the key must follow the content.
        let mut document = ProjectDocument::new("Key");
        let a = texture(&mut document, "fbm", vec![float("roughness", 0.5)]);
        let (_, first) = build(&document, a, None, 16).unwrap();
        document.resources.get_mut(&a).unwrap().parameters = vec![float("roughness", 0.7)];
        let (_, second) = build(&document, a, None, 16).unwrap();
        assert_ne!(first, second, "same edition, different content");
        document.resources.get_mut(&a).unwrap().parameters = vec![float("roughness", 0.5)];
        assert_eq!(build(&document, a, None, 16).unwrap().1, first);
        assert_ne!(
            build(&document, a, None, 32).unwrap().1,
            first,
            "size counts"
        );
    }

    #[test]
    fn a_shared_upstream_cannot_explode() {
        // 30 mixes, each taking the previous one twice: a legal DAG that
        // would expand to 2^30 nodes.
        let mut document = ProjectDocument::new("Wide");
        let mut previous = texture(&mut document, "constant", vec![]);
        for _ in 0..30 {
            let reference = |name: &str| Parameter {
                name: name.into(),
                value_type: "texture".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::Texture,
                    ids: vec![previous],
                },
            };
            previous = texture(
                &mut document,
                "mix",
                vec![reference("tex1"), reference("tex2")],
            );
        }
        let error = build(&document, previous, None, 16).unwrap_err();
        assert!(error.contains("too large"), "{error}");
    }

    #[test]
    fn encodings_follow_r4() {
        // r4: an explicit encoding (sRGB / srgb / linear / gamma G), else
        // the legacy gamma parameter, else.png sRGB and the rest linear;
        // float images (EXR) ignore it.
        let rule = |extension: &str, encoding: Option<&str>, gamma: Option<Gamma>, float: bool| {
            encoding_of(extension, encoding, gamma, float)
        };
        assert_eq!(rule("png", None, None, false), Ok(Encoding::Srgb));
        assert_eq!(rule("jpg", None, None, false), Ok(Encoding::Linear));
        assert_eq!(rule("tga", Some("srgb"), None, false), Ok(Encoding::Srgb));
        assert_eq!(
            rule("png", Some("linear"), None, false),
            Ok(Encoding::Linear)
        );
        assert_eq!(
            rule("png", Some("gamma 2.2"), None, false),
            Ok(Encoding::Gamma(2.2))
        );
        assert_eq!(
            rule("jpg", None, Some(Gamma::Legacy(true)), false),
            Ok(Encoding::Srgb)
        );
        assert_eq!(
            rule("png", None, Some(Gamma::Value(1.8)), false),
            Ok(Encoding::Gamma(1.8))
        );
        assert_eq!(rule("exr", Some("sRGB"), None, true), Ok(Encoding::Linear));
        assert!(rule("png", Some("rec709"), None, false).is_err());
    }

    #[test]
    fn a_blackbody_is_not_read_as_a_grey_value() {
        let mut document = ProjectDocument::new("Blackbody");
        let blackbody = Parameter {
            name: "tex1".into(),
            value_type: "blackbody".into(),
            color_space: ColorSpaceBinding::Inherit,
            values: ParameterValues::Floats(vec![FloatValue::Finite(6500.)]),
        };
        let id = texture(&mut document, "mix", vec![blackbody]);
        let (tree, _) = build(&document, id, None, 16).unwrap();
        let Texture::Mix { tex1, .. } = tree else {
            panic!("{tree:?}");
        };
        assert_eq!(tex1, Operand::Constant([0.; 3]), "the neutral fallback");
    }

    #[test]
    fn dots_read_inside_as_outside_and_outside_as_inside_like_r4() {
        let mut document = ProjectDocument::new("Dots");
        let declared = texture(
            &mut document,
            "dots",
            vec![float("inside", 0.25), float("outside", 0.75)],
        );
        let (tree, _) = build(&document, declared, None, 16).unwrap();
        let Texture::Dots {
            inside, outside, ..
        } = tree
        else {
            panic!("{tree:?}");
        };
        assert_eq!(outside, Operand::Constant([0.25; 3]));
        assert_eq!(inside, Operand::Constant([0.75; 3]));
        // The table's defaults: outside 1, inside 0.
        let bare = texture(&mut document, "dots", vec![]);
        let (tree, _) = build(&document, bare, None, 16).unwrap();
        let Texture::Dots {
            inside, outside, ..
        } = tree
        else {
            panic!("{tree:?}");
        };
        assert_eq!(outside, Operand::Constant([1.; 3]));
        assert_eq!(inside, Operand::Constant([0.; 3]));
    }

    #[test]
    fn float_imagemaps_read_one_value_per_texel_and_cache_per_lineage() {
        // One RGBA texel (linear, as.tga): RGB 0.2 0.4 0.6, alpha 0.5.
        let folder = tempfile::tempdir().unwrap();
        image::RgbaImage::from_raw(1, 1, vec![51, 102, 153, 128])
            .unwrap()
            .save(folder.path().join("a.tga"))
            .unwrap();
        let asset_bytes = std::fs::read(folder.path().join("a.tga")).unwrap();
        let mut document = ProjectDocument::new("Imagemap");
        let asset = project::Asset {
            id: project::AssetId::new(),
            location: "a.tga".into(),
            media_type: "image/x-tga".into(),
            sha256: format!("{:x}", Sha256::digest(&asset_bytes)),
            size: asset_bytes.len() as u64,
            origin: None,
        };
        let asset_id = asset.id;
        document.assets.insert(asset_id, asset);
        let filename = Parameter {
            name: "filename".into(),
            value_type: "string".into(),
            color_space: ColorSpaceBinding::Inherit,
            values: ParameterValues::Assets(vec![asset_id]),
        };
        let float = texture(&mut document, "imagemap", vec![filename.clone()]);
        let spectrum = texture(&mut document, "imagemap", vec![filename]);
        document
            .resources
            .get_mut(&spectrum)
            .unwrap()
            .texture_value_type = Some(TextureValueType::Spectrum);
        // Both read the same asset in one build: a mix of the two.
        let reference = |name: &str, id| Parameter {
            name: name.into(),
            value_type: "texture".into(),
            color_space: ColorSpaceBinding::Inherit,
            values: ParameterValues::Resources {
                kind: ResourceKind::Texture,
                ids: vec![id],
            },
        };
        let mix = texture(
            &mut document,
            "mix",
            vec![reference("tex1", float), reference("tex2", spectrum)],
        );
        let root = folder.path().to_str().unwrap();
        let (tree, _) = build(&document, mix, Some(root), 16).unwrap();
        let Texture::Mix {
            tex1: Operand::Texture(tex1),
            tex2: Operand::Texture(tex2),
            ..
        } = tree
        else {
            panic!("{tree:?}");
        };
        let pixel = |texture: &Texture| match texture {
            Texture::Image { image, .. } => image.pixels[0],
            other => panic!("{other:?}"),
        };
        assert_eq!(pixel(&tex1), [128. / 255.; 3], "float: the alpha");
        assert_eq!(pixel(&tex2), [51. / 255., 102. / 255., 153. / 255.]);
    }
    #[test]
    fn radiance_hdr_imagemaps_read_linear_float_texels() {
        // image 0.24's DynamicImage path decodes.hdr as gamma-2.2 Rgb8
        // clamped at 1.0, which then read as linear: 0.25 became ≈0.53.
        let folder = tempfile::tempdir().unwrap();
        let mut asset_bytes = Vec::new();
        image::codecs::hdr::HdrEncoder::new(&mut asset_bytes)
            .encode(&[image::Rgb([4.0f32, 0.25, 1.0])], 1, 1)
            .unwrap();
        std::fs::write(folder.path().join("sky.hdr"), &asset_bytes).unwrap();
        let mut document = ProjectDocument::new("Imagemap");
        let asset = project::Asset {
            id: project::AssetId::new(),
            location: "sky.hdr".into(),
            media_type: "image/vnd.radiance".into(),
            sha256: format!("{:x}", Sha256::digest(&asset_bytes)),
            size: asset_bytes.len() as u64,
            origin: None,
        };
        let asset_id = asset.id;
        document.assets.insert(asset_id, asset);
        let filename = Parameter {
            name: "filename".into(),
            value_type: "string".into(),
            color_space: ColorSpaceBinding::Inherit,
            values: ParameterValues::Assets(vec![asset_id]),
        };
        let spectrum = texture(&mut document, "imagemap", vec![filename]);
        document
            .resources
            .get_mut(&spectrum)
            .unwrap()
            .texture_value_type = Some(TextureValueType::Spectrum);
        let root = folder.path().to_str().unwrap();
        let (tree, _) = build(&document, spectrum, Some(root), 16).unwrap();
        let Texture::Image { image, .. } = tree else {
            panic!("{tree:?}");
        };
        let [r, g, b] = image.pixels[0];
        assert!((r - 4.0).abs() < 0.05, "light above 1.0 survives: {r}");
        assert!(
            (g - 0.25).abs() < 0.01,
            "HDR is linear, not gamma-encoded: {g}"
        );
        assert!((b - 1.0).abs() < 0.02, "{b}");
    }

    #[test]
    fn restored_image_previews_decode_payload_files_after_the_body_was_deleted() {
        let temporary = tempfile::tempdir().unwrap();
        let png = temporary.path().join("image.png");
        image::RgbImage::from_raw(1, 1, vec![200, 100, 50])
            .unwrap()
            .save(&png)
            .unwrap();
        let bytes = std::fs::read(&png).unwrap();
        let mut document = ProjectDocument::new_empty("Preview");
        let asset = project::Asset {
            id: project::AssetId::new(),
            location: "image.png".into(),
            media_type: "image/png".into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            size: bytes.len() as u64,
            origin: None,
        };
        document.assets.insert(asset.id, asset.clone());
        let id = texture(
            &mut document,
            "imagemap",
            vec![Parameter {
                name: "filename".into(),
                value_type: "string".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Assets(vec![asset.id]),
            }],
        );
        let path = temporary.path().join("project");
        let mut store = project::FolderStore::create_v3(
            &path,
            document.id,
            "Preview",
            project::Revision::initial(),
        )
        .unwrap();
        store.import_asset(&asset, &bytes[..], || false).unwrap();
        let mut session = project::ProjectSession::new(document).unwrap();
        session
            .enable_temporary_payload_staging(&path.join("Assets"))
            .unwrap();
        store.save(&session.snapshot()).unwrap();
        session
            .apply(
                &session.revision(),
                &[
                    project::Edit::DeleteItem {
                        item: project::ItemId::Resource(id),
                    },
                    project::Edit::DeleteItem {
                        item: project::ItemId::Asset(asset.id),
                    },
                ],
            )
            .unwrap();
        store.save(&session.snapshot()).unwrap();
        session.undo(&session.revision()).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(
            snapshot
                .payload_by(asset.id)
                .unwrap()
                .path()
                .extension()
                .unwrap(),
            "payload"
        );
        assert!(tree_from_snapshot(&snapshot, id, path.join("Assets").to_str()).is_ok());
    }
}
