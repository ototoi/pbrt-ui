//! PLY reader for ASCII and common binary vertex/face meshes.
use crate::error::SceneParseError;
mod bounded;
pub use bounded::{
    PlyValidationError, PlyValidationLimits, validate_ply_file, validate_ply_gzip_file,
};

#[derive(Clone)]
enum Property {
    Scalar(String, String),
}
fn error(path: &str, message: impl ToString) -> SceneParseError {
    SceneParseError::Ply {
        path: path.into(),
        message: message.to_string(),
    }
}
fn scalar_size(ty: &str) -> Result<usize, SceneParseError> {
    match ty {
        "char" | "int8" | "uchar" | "uint8" => Ok(1),
        "short" | "int16" | "ushort" | "uint16" => Ok(2),
        "int" | "int32" | "uint" | "uint32" | "float" | "float32" => Ok(4),
        "double" | "float64" | "int64" | "uint64" => Ok(8),
        _ => Err(error(
            "<header>",
            format!("unsupported PLY scalar type {ty}"),
        )),
    }
}
fn decode_scalar(ty: &str, bytes: &[u8], little: bool) -> Result<f64, ()> {
    let n = scalar_size(ty).map_err(|_| ())?;
    if bytes.len() != n {
        return Err(());
    }
    let mut b = [0u8; 8];
    if little {
        b[..n].copy_from_slice(bytes)
    } else {
        for i in 0..n {
            b[i] = bytes[n - 1 - i]
        }
    }
    Ok(match ty {
        "char" | "int8" => bytes[0] as i8 as f64,
        "uchar" | "uint8" => bytes[0] as f64,
        "short" | "int16" => i16::from_ne_bytes(b[..2].try_into().unwrap()) as f64,
        "ushort" | "uint16" => u16::from_ne_bytes(b[..2].try_into().unwrap()) as f64,
        "int" | "int32" => i32::from_ne_bytes(b[..4].try_into().unwrap()) as f64,
        "uint" | "uint32" => u32::from_ne_bytes(b[..4].try_into().unwrap()) as f64,
        "float" | "float32" => f32::from_ne_bytes(b[..4].try_into().unwrap()) as f64,
        "double" | "float64" => f64::from_ne_bytes(b),
        "int64" => i64::from_ne_bytes(b) as f64,
        "uint64" => u64::from_ne_bytes(b) as f64,
        _ => return Err(()),
    })
}
fn read_one(
    bytes: &[u8],
    cursor: &mut usize,
    ty: &str,
    little: bool,
    path: &str,
) -> Result<f64, SceneParseError> {
    let n = scalar_size(ty).map_err(|e| error(path, e))?;
    let end = cursor
        .checked_add(n)
        .ok_or_else(|| error(path, "offset overflow"))?;
    let part = bytes
        .get(*cursor..end)
        .ok_or_else(|| error(path, "truncated binary PLY data"))?;
    *cursor = end;
    decode_scalar(ty, part, little).map_err(|_| error(path, "invalid PLY scalar"))
}

/// A parsed PLY header: where the body starts and what it holds.
struct Header {
    format: String,
    vertices: usize,
    faces: usize,
    vertex_props: Vec<Property>,
    face_prop: Option<(String, String, String)>,
    end: usize,
}

fn parse_header(path: &str, bytes: &[u8]) -> Result<Header, SceneParseError> {
    let header_end = bytes
        .windows(11)
        .position(|w| w == b"end_header\n")
        .map(|p| p + 11)
        .or_else(|| {
            bytes
                .windows(12)
                .position(|w| w == b"end_header\r\n")
                .map(|p| p + 12)
        })
        .ok_or_else(|| error(path, "invalid PLY header"))?;
    let text = std::str::from_utf8(&bytes[..header_end]).map_err(|e| error(path, e))?;
    let mut lines = text.lines();
    if lines.next() != Some("ply") {
        return Err(error(path, "missing ply header"));
    }
    let mut format = "";
    let (mut nv, mut nf) = (0usize, 0usize);
    let (mut vertex_props, mut face_prop) = (Vec::new(), None);
    let (mut element, mut ended) = (String::new(), false);
    for line in lines.by_ref() {
        let p = line.split_whitespace().collect::<Vec<_>>();
        match p.as_slice() {
            ["format", f, _] => format = *f,
            ["element", name, count] => {
                element = (*name).to_string();
                let n = count
                    .parse()
                    .map_err(|_| error(path, "invalid element count"))?;
                match *name {
                    "vertex" => nv = n,
                    "face" => nf = n,
                    _ => {}
                }
            }
            ["property", "list", count, item, name] if element == "face" => {
                face_prop = Some((
                    (*count).to_string(),
                    (*item).to_string(),
                    (*name).to_string(),
                ))
            }
            ["property", ty, name] if element == "vertex" => {
                vertex_props.push(Property::Scalar((*ty).into(), (*name).into()))
            }
            ["end_header"] => {
                ended = true;
                break;
            }
            _ => {}
        }
    }
    if !ended {
        return Err(error(path, "missing end_header"));
    }
    let has = |wanted: &str| {
        vertex_props
            .iter()
            .any(|p| matches!(p, Property::Scalar(_, n) if n == wanted))
    };
    if !has("x") || !has("y") || !has("z") {
        return Err(error(path, "vertex properties must contain x, y, z"));
    }
    Ok(Header {
        format: format.to_owned(),
        vertices: nv,
        faces: nf,
        vertex_props,
        face_prop,
        end: header_end,
    })
}

/// What a PLY header says, without reading the body (used for the Mesh
/// Viewer's and `project.get_asset`'s statistics).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlyInfo {
    pub vertices: usize,
    pub faces: usize,
    /// nx / ny / nz vertex properties.
    pub normals: bool,
    /// u / v, s / t, texture_u / texture_v or texture_s / texture_t.
    pub uvs: bool,
    pub format: String,
}

/// `ply_info` for a file, reading only its first 64 KiB (a header never
/// needs more), so a multi-gigabyte asset is never loaded for statistics.
pub fn ply_info_from_file(path: &std::path::Path) -> Result<PlyInfo, SceneParseError> {
    use std::io::{Read, Seek, SeekFrom};
    let name = path.to_string_lossy();
    let mut file = std::fs::File::open(path).map_err(|e| error(&name, e))?;
    let mut magic = [0; 2];
    let is_gzip = file.read_exact(&mut magic).is_ok() && magic == [0x1f, 0x8b];
    file.seek(SeekFrom::Start(0)).map_err(|e| error(&name, e))?;
    let mut head = Vec::new();
    if is_gzip {
        flate2::read::MultiGzDecoder::new(file)
            .take(64 * 1024)
            .read_to_end(&mut head)
            .map_err(|e| error(&name, e))?;
    } else {
        file.take(64 * 1024)
            .read_to_end(&mut head)
            .map_err(|e| error(&name, e))?;
    }
    ply_info(&name, &head)
}

/// Read a PLY header; `bytes` need only reach `end_header`.
pub fn ply_info(path: &str, bytes: &[u8]) -> Result<PlyInfo, SceneParseError> {
    let header = parse_header(path, bytes)?;
    let has = |wanted: &str| {
        header
            .vertex_props
            .iter()
            .any(|p| matches!(p, Property::Scalar(_, n) if n == wanted))
    };
    Ok(PlyInfo {
        vertices: header.vertices,
        faces: header.faces,
        normals: has("nx") && has("ny") && has("nz"),
        uvs: UV_NAMES.iter().any(|(u, v)| has(u) && has(v)),
        format: header.format,
    })
}

/// pbrt's PLY reader takes these UV spellings.
const UV_NAMES: [(&str, &str); 4] = [
    ("u", "v"),
    ("s", "t"),
    ("texture_u", "texture_v"),
    ("texture_s", "texture_t"),
];

pub type PlyMesh = (Vec<[f64; 3]>, Vec<u32>, Option<Vec<[f32; 2]>>);

pub fn read_ply(path: &str, bytes: &[u8]) -> Result<PlyMesh, SceneParseError> {
    let Header {
        format,
        vertices: nv,
        faces: nf,
        vertex_props,
        face_prop,
        end: header_end,
    } = parse_header(path, bytes)?;
    let format = format.as_str();
    let declared = |wanted: &str| {
        vertex_props
            .iter()
            .any(|Property::Scalar(_, name)| name == wanted)
    };
    let uv_names = UV_NAMES
        .iter()
        .find(|(u, v)| declared(u) && declared(v))
        .copied();
    let is_u = |name: &str| uv_names.is_some_and(|(u, _)| u == name);
    let is_v = |name: &str| uv_names.is_some_and(|(_, v)| v == name);
    let mut uvs = Vec::with_capacity(if uv_names.is_some() { nv } else { 0 });
    let mut points = Vec::with_capacity(nv);
    let mut indices = Vec::new();
    match format {
        "ascii" => {
            let body = std::str::from_utf8(&bytes[header_end..]).map_err(|e| error(path, e))?;
            let mut tokens = body.split_whitespace();
            for _ in 0..nv {
                let mut xyz = [0.; 3];
                let mut uv = [0.; 2];
                for prop in &vertex_props {
                    let Property::Scalar(ty, name) = prop;
                    let token = tokens
                        .next()
                        .ok_or_else(|| error(path, "truncated vertex data"))?;
                    let value = token.parse::<f64>().map_err(|e| error(path, e))?;
                    if name == "x" {
                        xyz[0] = value
                    } else if name == "y" {
                        xyz[1] = value
                    } else if name == "z" {
                        xyz[2] = value
                    } else if is_u(name) {
                        uv[0] = value as f32
                    } else if is_v(name) {
                        uv[1] = value as f32
                    }
                    let _ = ty;
                }
                points.push(xyz);
                if uv_names.is_some() {
                    uvs.push(uv);
                }
            }
            read_faces_ascii(path, &mut tokens, nf, face_prop.as_ref(), nv, &mut indices)?;
        }
        "binary_little_endian" | "binary_big_endian" => {
            let little = format == "binary_little_endian";
            let all = &bytes[header_end..];
            let mut cursor = 0;
            for _ in 0..nv {
                let mut xyz = [0.; 3];
                let mut uv = [0.; 2];
                for prop in &vertex_props {
                    let Property::Scalar(ty, name) = prop;
                    let value = read_one(all, &mut cursor, ty, little, path)?;
                    if name == "x" {
                        xyz[0] = value
                    } else if name == "y" {
                        xyz[1] = value
                    } else if name == "z" {
                        xyz[2] = value
                    } else if is_u(name) {
                        uv[0] = value as f32
                    } else if is_v(name) {
                        uv[1] = value as f32
                    }
                }
                points.push(xyz);
                if uv_names.is_some() {
                    uvs.push(uv);
                }
            }
            let (count_ty, item_ty, _) = face_prop
                .as_ref()
                .ok_or_else(|| error(path, "face vertex index list is missing"))?;
            for _ in 0..nf {
                let count = read_one(all, &mut cursor, count_ty, little, path)? as usize;
                if !(3..=1_000_000).contains(&count) {
                    return Err(error(path, "invalid face vertex count"));
                }
                let mut face = Vec::with_capacity(count);
                for _ in 0..count {
                    let i = read_one(all, &mut cursor, item_ty, little, path)?;
                    if i < 0. || i >= nv as f64 || i.fract() != 0. {
                        return Err(error(path, "face index outside vertex array"));
                    }
                    face.push(i as u32)
                }
                triangulate(&face, &mut indices)
            }
        }
        other => return Err(error(path, format!("unsupported PLY format {other}"))),
    }
    if points.iter().flatten().any(|v| !v.is_finite()) || indices.is_empty() {
        return Err(error(path, "PLY contains no finite triangle mesh"));
    }
    Ok((points, indices, uv_names.map(|_| uvs)))
}
fn triangulate(face: &[u32], out: &mut Vec<u32>) {
    for i in 1..face.len() - 1 {
        out.extend([face[0], face[i], face[i + 1]])
    }
}
fn read_faces_ascii<'a>(
    path: &str,
    tokens: &mut impl Iterator<Item = &'a str>,
    count: usize,
    prop: Option<&(String, String, String)>,
    vertex_count: usize,
    out: &mut Vec<u32>,
) -> Result<(), SceneParseError> {
    let (count_ty, item_ty, _) =
        prop.ok_or_else(|| error(path, "face vertex index list is missing"))?;
    let _ = (count_ty, item_ty);
    for _ in 0..count {
        let n = tokens
            .next()
            .ok_or_else(|| error(path, "truncated face data"))?
            .parse::<usize>()
            .map_err(|e| error(path, e))?;
        if !(3..=1_000_000).contains(&n) {
            return Err(error(path, "invalid face vertex count"));
        }
        let mut face = Vec::with_capacity(n);
        for _ in 0..n {
            let index = tokens
                .next()
                .ok_or_else(|| error(path, "truncated face data"))?
                .parse::<u32>()
                .map_err(|e| error(path, e))?;
            if index as usize >= vertex_count {
                return Err(error(path, "face index outside vertex array"));
            }
            face.push(index)
        }
        triangulate(&face, out)
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ascii_quad_and_triangulates() {
        let bytes = b"ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n";
        let (points, indices, _) = read_ply("quad.ply", bytes).unwrap();
        assert_eq!(points.len(), 4);
        assert_eq!(indices, [0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn reads_binary_little_endian_vertex_and_face_records() {
        let mut bytes = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n".to_vec();
        for point in [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
            for value in point {
                bytes.extend(value.to_le_bytes());
            }
        }
        bytes.push(3);
        for index in [0i32, 1, 2] {
            bytes.extend(index.to_le_bytes());
        }
        let (points, indices, _) = read_ply("triangle.ply", &bytes).unwrap();
        assert_eq!(points[1], [1., 0., 0.]);
        assert_eq!(indices, [0, 1, 2]);
    }
    #[test]
    fn reads_uvs_in_each_of_pbrts_spellings_and_only_as_pairs() {
        let scene = |u: &str, v: &str| {
            format!(
                "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float {u}\nproperty float y\nproperty float z\nproperty float {v}\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0.25 0 0 0.75\n1 0.5 0 0 0.125\n0 1 1 0 0\n3 0 1 2\n"
            )
        };
        for (u, v) in [
            ("u", "v"),
            ("s", "t"),
            ("texture_u", "texture_v"),
            ("texture_s", "texture_t"),
        ] {
            let (_, _, uvs) = read_ply("uv.ply", scene(u, v).as_bytes()).unwrap();
            assert_eq!(
                uvs.unwrap(),
                [[0.25, 0.75], [0.5, 0.125], [1., 0.]],
                "{u}/{v}"
            );
        }
        let (_, _, uvs) = read_ply("uv.ply", scene("u", "t").as_bytes()).unwrap();
        assert_eq!(uvs, None);
    }

    #[test]
    fn info_reads_only_the_header() {
        // Counts and attributes are available without reading the body.
        let header = b"ply\nformat binary_little_endian 1.0\nelement vertex 1000\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\nproperty float ny\nproperty float nz\nproperty float u\nproperty float v\nelement face 1996\nproperty list uchar int vertex_indices\nend_header\n\x00\x01";
        let info = ply_info("mesh.ply", header).unwrap();
        assert_eq!(
            info,
            PlyInfo {
                vertices: 1000,
                faces: 1996,
                normals: true,
                uvs: true,
                format: "binary_little_endian".into(),
            }
        );
        // s/t also count as UVs; no normals here.
        let plain = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float s\nproperty float t\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n";
        let info = ply_info("plain.ply", plain).unwrap();
        assert!(!info.normals && info.uvs);
        let texture_uv = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float texture_u\nproperty float texture_v\nend_header\n";
        assert!(ply_info("texture.ply", texture_uv).unwrap().uvs);
        // A broken header is an error, not zero counts.
        assert!(ply_info("broken.ply", b"ply\nformat ascii 1.0\nelement vertex 3\n").is_err());
        assert!(ply_info("notply.ply", b"hello\nend_header\n").is_err());
    }
}
