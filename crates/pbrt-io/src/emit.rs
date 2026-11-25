//! Single-source emission: the ONE node traversal that decides what a project
//! snapshot exports. Both consumers — the scene.pbrt text writer and the
//! direct-preview program —
//! subscribe to the same event stream (P1/P5 parity by construction).
use crate::{Error, check, invalid, unsupported};
use project::{
    ActiveSamples, AssetId, ConfigurationKind, EntityKind, EvaluatedScene, FloatValue,
    MaterialBinding, NodeId, NodeKind, OptionValue, ParameterValues, ProjectDocument, ProjectStore,
    Resource, ResourceId, ResourceKind, SceneView, ScopeKind, StateOperation, TextureValueType,
    TransformOperation,
};
use protocol::EntityId;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

/// One emitted token in its scene.pbrt spelling. `Quoted` holds the unescaped
/// text; the lexeme re-quotes it.
pub(crate) enum Token {
    Word(String),
    Quoted(String),
    Number(String),
}
impl Token {
    /// The exact bytes this token takes in scene.pbrt (also the input the
    /// program sink re-lexes).
    pub(crate) fn lexeme(&self) -> String {
        match self {
            Token::Word(text) | Token::Number(text) => text.clone(),
            Token::Quoted(text) => quoted(text),
        }
    }
}

pub(crate) fn name(id: project::ResourceId) -> String {
    format!("resource_{id}")
}

pub(crate) fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) fn scalar(value: &FloatValue) -> String {
    match value {
        FloatValue::Finite(v) => v.to_string(),
        FloatValue::PositiveInfinity => "inf".into(),
        FloatValue::NegativeInfinity => "-inf".into(),
        FloatValue::Nan => "nan".into(),
    }
}

/// A SpectrumData resource's samples: inline float tables read directly,
/// `.array` tables read from the asset the caller resolves (`store` or
/// snapshot-aware `asset_path`). Equal counts are a validation invariant;
/// a project that lost it is refused here instead of exporting halves.
/// The swatch colour calculation (`project.spectrum_color`) also reads through
/// this function, so both paths use the same implementation.
pub fn spectrum_samples(
    resource: &Resource,
    open: impl Fn(AssetId) -> crate::Result<std::path::PathBuf>,
) -> crate::Result<(Vec<f64>, Vec<f64>)> {
    let read = |name: &str| -> crate::Result<Vec<f64>> {
        let parameter = resource
            .parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .ok_or_else(|| invalid(format!("spectrum data resource has no {name} sample table")))?;
        match &parameter.values {
            ParameterValues::Floats(values) => values
                .iter()
                .map(|value| match value {
                    FloatValue::Finite(n) if n.is_finite() => Ok(*n),
                    _ => Err(invalid("spectrum samples must be finite")),
                })
                .collect(),
            ParameterValues::Array {
                asset,
                encoding,
                count,
            } => {
                if *encoding != project::ArrayEncoding::F64Le {
                    return Err(invalid("spectrum data tables must hold f64"));
                }
                let path = open(*asset)?;
                let mut file = std::io::BufReader::new(std::fs::File::open(path)?);
                let mut out = Vec::with_capacity(*count as usize);
                for _ in 0..*count {
                    let mut bytes = [0; 8];
                    file.read_exact(&mut bytes)?;
                    out.push(f64::from_le_bytes(bytes));
                }
                Ok(out)
            }
            _ => Err(invalid("spectrum data tables must be floats")),
        }
    };
    let wavelengths = read("wavelengths")?;
    let values = read("values")?;
    if wavelengths.len() != values.len() {
        return Err(invalid("spectrum data sample counts differ"));
    }
    Ok((wavelengths, values))
}

/// A SpectrumData curve longer than the preview transfer's pair budget
/// resamples onto the 380–780 nm decade grid (20 nm, 21 samples). Each
/// sample is the mean of r4's piecewise-linear read over its 20 nm bin
/// (outside the curve's span reads 0, as r4's `PiecewiseLinearSpectrum`
/// does), so a peak narrower than the grid keeps its energy instead of
/// falling between two samples. The swatches then see the whole visible
/// band, and the curve sits well under `MAX_PREVIEW_PARAMETER_VALUES` —
/// the standard truncation never fires. The text export keeps every
/// sample: only the preview program sinks here, which expands into the
/// program, the export writes its spds file).
fn decimated_preview_curve(curve: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    const MAX_CURVE_PAIRS: usize = protocol::MAX_PREVIEW_PARAMETER_VALUES / 2;
    if curve.len() <= MAX_CURVE_PAIRS {
        return curve;
    }
    let bin_mean = |from: f64, to: f64| {
        let mut integral = 0.;
        for pair in curve.windows(2) {
            let ((l0, v0), (l1, v1)) = (pair[0], pair[1]);
            let (a, b) = (l0.max(from), l1.min(to));
            if b <= a {
                continue;
            }
            let at = |lambda: f64| v0 + (lambda - l0) / (l1 - l0) * (v1 - v0);
            integral += 0.5 * (at(a) + at(b)) * (b - a);
        }
        integral / (to - from)
    };
    (0..=20)
        .map(|index| {
            let lambda = 380. + 20. * index as f64;
            (lambda, bin_mean(lambda - 10., lambda + 10.))
        })
        .collect()
}

/// Emission events. The traversal calls exactly these; sinks translate.
/// `checkpoint` runs at every node entry and pins the node as the owner of
/// the next emitted directive (the origin used for picks and stable ids).
pub(crate) trait Emission {
    fn checkpoint(&mut self, node: NodeId) -> Result<(), Error>;
    /// Begin a directive. `bracket` mirrors the matrix transforms' `[ … ]`
    /// positional wrapper.
    fn begin(&mut self, name: &str, bracket: bool) -> Result<(), Error>;
    fn positional(&mut self, token: &Token) -> Result<(), Error>;
    /// The packed `"string type" ["…"]` header of Make* directives.
    fn builtin_type(&mut self, implementation: &str) -> Result<(), Error>;
    fn begin_parameter(&mut self, value_type: &str, name: &str) -> Result<(), Error>;
    fn parameter_value(&mut self, token: &Token) -> Result<(), Error>;
    fn end_parameter(&mut self) -> Result<(), Error>;
    /// The packed trailing `"string filename" ["render.exr"]` Film default.
    fn film_filename(&mut self) -> Result<(), Error>;
    fn end_directive(&mut self) -> Result<(), Error>;
    /// Whether a `spectrum` parameter written from a SpectrumData resource
    /// reference expands into interleaved λ/value values instead of
    /// quoting a written spd file: the direct-preview program, so
    /// the Lit resolver reads the pairs without resolving any file. The
    /// text export writes `spds/<name>.spd` and quotes its path.
    fn expand_spectrum_samples(&self) -> bool {
        false
    }
    /// Whether a MakeNamedMedium keeps its parameters, like the text
    /// export. The direct-preview program answers false: the preview
    /// never draws media, and a grid medium's bulk arrays can hold
    /// millions of values, so the program keeps the name and type only.
    fn medium_parameters(&self) -> bool {
        true
    }
}

/// The node traversal. `document`, `evaluated`, `store` and `assets` are the
/// same inputs `export_project` prepared for the old Writer.
pub(crate) struct Emitter<'a, F: Fn() -> bool> {
    pub snapshot: Option<&'a project::ProjectSnapshot>,
    pub view: SceneView<'a>,
    pub evaluated: &'a EvaluatedScene,
    pub store: &'a ProjectStore,
    pub assets: &'a BTreeMap<AssetId, String>,
    pub names: &'a BTreeMap<ResourceId, String>,
    /// Exported spd paths for SpectrumData resources, as a `spectrum`
    /// parameter references them. The text export stages the files; the
    /// program callers pass an empty map and keep the resource-name
    /// fallback until the direct-preview expansion lands.
    pub spectra: &'a BTreeMap<ResourceId, String>,
    pub cancelled: &'a F,
}

/// Derive export names from labels separately for each export.
pub(crate) fn resource_names(document: &ProjectDocument) -> BTreeMap<ResourceId, String> {
    let mut used = BTreeMap::<u8, std::collections::BTreeSet<String>>::new();
    let mut names = BTreeMap::new();
    let mut resources = document.resources.values().collect::<Vec<_>>();
    resources.sort_by(|a, b| {
        kind_key(a.kind)
            .cmp(&kind_key(b.kind))
            .then(a.location.cmp(&b.location))
            .then(a.id.cmp(&b.id))
    });
    for resource in resources {
        let kind = kind_key(resource.kind);
        let base = sanitize_name(&resource.label);
        let base = if base.is_empty() {
            format!("{}_{}", kind_name(resource.kind), names.len() + 1)
        } else {
            base
        };
        let mut candidate = base.clone();
        let mut number = 1;
        while !used.entry(kind).or_default().insert(candidate.clone()) {
            candidate = format!("{base}_{number}");
            number += 1;
        }
        names.insert(resource.id, candidate);
    }
    names
}
/// A resource naming others of its kind, such as a mix and its materials, comes
/// after them: pbrt resolves those names as it reads. Otherwise keeps `sorted`.
fn in_dependency_order<'a>(
    sorted: Vec<&'a Resource>,
    document: &'a ProjectDocument,
) -> Vec<&'a Resource> {
    fn visit<'a>(
        resource: &'a Resource,
        document: &'a ProjectDocument,
        seen: &mut std::collections::BTreeSet<ResourceId>,
        out: &mut Vec<&'a Resource>,
    ) {
        if !seen.insert(resource.id) {
            return;
        }
        for parameter in &resource.parameters {
            if let ParameterValues::Resources { kind, ids } = &parameter.values {
                if *kind == resource.kind {
                    for id in ids {
                        if let Some(child) = document.resources.get(id) {
                            visit(child, document, seen, out);
                        }
                    }
                }
            }
        }
        out.push(resource);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(sorted.len());
    for resource in sorted {
        visit(resource, document, &mut seen, &mut out);
    }
    out
}
fn kind_key(kind: ResourceKind) -> u8 {
    match kind {
        ResourceKind::Texture => 0,
        ResourceKind::Material => 1,
        ResourceKind::Medium => 2,
        ResourceKind::ObjectDefinition => 3,
        ResourceKind::SpectrumData => 4,
    }
}
fn kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Texture => "texture",
        ResourceKind::Material => "material",
        ResourceKind::Medium => "medium",
        ResourceKind::ObjectDefinition => "object",
        ResourceKind::SpectrumData => "spectrum",
    }
}
fn sanitize_name(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The imagemaps without an image (New-menu ones before an image is chosen)
/// that `view`'s export leaves out, so the scene still renders. Every
/// resource is written, and every node of this scene and of the object
/// definitions; a reference from any of them is an error naming the
/// texture, rather than a Texture pbrt would reject. Another scene's node
/// is not written, so its reference does not count.
pub(crate) fn unset_imagemaps<'a>(
    view: &SceneView<'a>,
    references: &project::ReferenceIndex,
) -> Result<Vec<&'a Resource>, Error> {
    let mut skipped = Vec::new();
    for resource in view.document.resources.values() {
        if resource.implementation != "imagemap"
            || resource.parameters.iter().any(|p| p.name == "filename")
        {
            continue;
        }
        let written = references
            .references(project::ItemId::Resource(resource.id))
            .iter()
            .any(|at| at.scene.is_none() || at.scene == Some(view.scene.id));
        if written {
            return Err(Error::ImagemapWithoutImage {
                resource: resource.id,
                label: resource.label.clone(),
            });
        }
        skipped.push(resource);
    }
    Ok(skipped)
}

impl<'a, F: Fn() -> bool> Emitter<'a, F> {
    fn resource_name(&self, id: ResourceId) -> String {
        self.names.get(&id).cloned().unwrap_or_else(|| name(id))
    }

    /// Emit the selected camera after restoring state captured at its source
    /// declaration. PBRT consumes Options sequentially, so it must be last.
    fn selected_camera(&self, sink: &mut dyn Emission, id: NodeId) -> Result<(), Error> {
        let node = self
            .view
            .node(id)
            .ok_or_else(|| invalid("selected Camera is missing"))?;
        let NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation,
            parameters,
            pose,
            ..
        } = &node.data
        else {
            return Err(invalid("selected render node is not a Camera"));
        };
        let state = self
            .evaluated
            .nodes
            .get(&id)
            .ok_or_else(|| invalid("selected Camera state is unavailable"))?;
        let final_state = self
            .view
            .scene
            .options
            .last()
            .and_then(|option| self.evaluated.nodes.get(option))
            .cloned()
            .unwrap_or_default();
        if state.times != final_state.times {
            sink.begin("TransformTimes", false)?;
            for time in state.times {
                sink.positional(&Token::Number(format!("{time}")))?;
            }
            sink.end_directive()?;
        }
        // A posed camera writes its own inverse pose — one transform when
        // static, a per-sample split with `ActiveTransform` choreography when
        // animated. No pose (a saved project) keeps reproducing the evaluated
        // declaration state exactly as the parser saw it.
        match pose.as_ref() {
            Some(pose) => self.emit_pose(sink, pose)?,
            None => self.declaration_transform(sink, state)?,
        }
        if (state.inside_medium, state.outside_medium)
            != (final_state.inside_medium, final_state.outside_medium)
        {
            self.medium_interface(sink, state.inside_medium, state.outside_medium)?;
        }
        if state.color_space != final_state.color_space {
            self.color_space(sink, &state.color_space)?;
        }
        sink.checkpoint(id)?;
        sink.begin("Camera", false)?;
        sink.positional(&Token::Quoted(implementation.clone()))?;
        self.parameters(sink, parameters, &state.color_space)?;
        sink.end_directive()?;
        // Camera is moved to the end of Options, so put the sequential state
        // back exactly as it was for the WorldBegin and following world.
        if (state.inside_medium, state.outside_medium)
            != (final_state.inside_medium, final_state.outside_medium)
        {
            self.medium_interface(sink, final_state.inside_medium, final_state.outside_medium)?;
        }
        if state.color_space != final_state.color_space {
            self.color_space(sink, &final_state.color_space)?;
        }
        if state.times != final_state.times {
            sink.begin("TransformTimes", false)?;
            for time in final_state.times {
                sink.positional(&Token::Number(format!("{time}")))?;
            }
            sink.end_directive()?;
        }
        Ok(())
    }

    fn medium_interface(
        &self,
        sink: &mut dyn Emission,
        inside: Option<ResourceId>,
        outside: Option<ResourceId>,
    ) -> Result<(), Error> {
        sink.begin("MediumInterface", false)?;
        sink.positional(&Token::Quoted(
            inside.map(|id| self.resource_name(id)).unwrap_or_default(),
        ))?;
        sink.positional(&Token::Quoted(
            outside.map(|id| self.resource_name(id)).unwrap_or_default(),
        ))?;
        sink.end_directive()
    }

    fn color_space(&self, sink: &mut dyn Emission, color_space: &str) -> Result<(), Error> {
        sink.begin("ColorSpace", false)?;
        sink.positional(&Token::Quoted(color_space.into()))?;
        sink.end_directive()
    }

    /// Emit the options block (including the default Film injection), then
    /// WorldBegin, canonical resource declarations, and the world tree.
    pub fn whole_scene(&self, sink: &mut dyn Emission) -> Result<(), Error> {
        let scene = self.view.scene;
        // The selected camera is emitted last by `selected_camera` (last
        // Camera wins in PBRT). Emitting its node here as well would put two
        // `Camera` directives for one node in the text, so every
        // export/import round trip would add another camera node.
        for id in &scene.options {
            if scene.render_camera == Some(*id) {
                continue;
            }
            self.node(sink, *id)?;
        }
        if let Some(camera) = scene.render_camera {
            self.selected_camera(sink, camera)?;
        }
        let has_film = scene.options.iter().any(|id| {
            matches!(
                self.view.node(*id).map(|n| &n.data),
                Some(NodeKind::Configuration {
                    configuration: ConfigurationKind::Film,
                    ..
                })
            )
        });
        if !has_film {
            sink.begin("Film", false)?;
            sink.positional(&Token::Quoted("rgb".into()))?;
            sink.film_filename()?;
            sink.end_directive()?;
        }
        sink.checkpoint(scene.world)?;
        sink.begin("WorldBegin", false)?;
        sink.end_directive()?;
        self.resources(sink)?;
        self.children(sink, scene.world)
    }

    fn resources(&self, sink: &mut dyn Emission) -> Result<(), Error> {
        let mut resources = self.view.document.resources.values().collect::<Vec<_>>();
        resources.sort_by(|a, b| {
            kind_key(a.kind)
                .cmp(&kind_key(b.kind))
                .then(a.location.cmp(&b.location))
                .then(a.id.cmp(&b.id))
        });
        // A snapshot's index is built once and shared with the export's
        // warnings; a bare document (tests, fixtures) builds its own.
        let references = match self.snapshot {
            Some(snapshot) => snapshot.reference_index(),
            None => std::sync::Arc::new(self.view.document.reference_index()),
        };
        let skipped = unset_imagemaps(&self.view, &references)?
            .iter()
            .map(|resource| resource.id)
            .collect::<std::collections::BTreeSet<_>>();
        for resource in in_dependency_order(resources, self.view.document) {
            check(self.cancelled)?;
            if skipped.contains(&resource.id) {
                continue;
            }
            self.resource(sink, resource)?;
        }
        Ok(())
    }

    fn resource(&self, sink: &mut dyn Emission, r: &Resource) -> Result<(), Error> {
        match r.kind {
            ResourceKind::ObjectDefinition => self.object_definition(sink, r),
            ResourceKind::Material => {
                let declared = r.declared_in.as_ref();
                if declared.is_some_and(|d| d.transform.is_some()) {
                    return Err(invalid("material captures transform state"));
                }
                let graphics = declared.and_then(|d| d.graphics.as_ref());
                if let Some(graphics) = graphics {
                    sink.begin("AttributeBegin", false)?;
                    sink.end_directive()?;
                    self.graphics_state(sink, graphics)?;
                }
                sink.begin("MakeNamedMaterial", false)?;
                sink.positional(&Token::Quoted(self.resource_name(r.id)))?;
                sink.builtin_type(&r.implementation)?;
                self.parameters(
                    sink,
                    &r.parameters,
                    &self.evaluated.resources[&r.id].color_space,
                )?;
                sink.end_directive()?;
                if graphics.is_some() {
                    sink.begin("AttributeEnd", false)?;
                    sink.end_directive()?;
                }
                Ok(())
            }
            ResourceKind::Texture | ResourceKind::Medium => {
                let wrap = r
                    .declared_in
                    .as_ref()
                    .is_some_and(|d| d.transform.is_some());
                if wrap {
                    sink.begin("AttributeBegin", false)?;
                    sink.end_directive()?;
                    self.declared_transform(sink, r)?;
                }
                if r.kind == ResourceKind::Texture {
                    sink.begin("Texture", false)?;
                    sink.positional(&Token::Quoted(self.resource_name(r.id)))?;
                    sink.positional(&Token::Quoted(match r.texture_value_type.unwrap() {
                        TextureValueType::Float => "float".into(),
                        TextureValueType::Spectrum => "spectrum".into(),
                    }))?;
                    sink.positional(&Token::Quoted(r.implementation.clone()))?;
                } else {
                    sink.begin("MakeNamedMedium", false)?;
                    sink.positional(&Token::Quoted(self.resource_name(r.id)))?;
                    sink.builtin_type(&r.implementation)?;
                }
                if r.kind == ResourceKind::Texture || sink.medium_parameters() {
                    self.parameters(
                        sink,
                        &r.parameters,
                        &self.evaluated.resources[&r.id].color_space,
                    )?;
                }
                sink.end_directive()?;
                if wrap {
                    sink.begin("AttributeEnd", false)?;
                    sink.end_directive()?;
                }
                Ok(())
            }
            // Spectrum data is emitted where it is referenced: the text
            // export writes an spd file per reference, the program
            // expands the samples. The resource itself declares nothing.
            ResourceKind::SpectrumData => Ok(()),
        }
    }

    fn object_definition(&self, sink: &mut dyn Emission, r: &Resource) -> Result<(), Error> {
        let root = r
            .definition_scope
            .ok_or_else(|| invalid("object definition has no scope"))?;
        sink.begin("AttributeBegin", false)?;
        sink.end_directive()?;
        self.declared_transform(sink, r)?;
        if let Some(graphics) = r.declared_in.as_ref().and_then(|d| d.graphics.as_ref()) {
            self.graphics_state(sink, graphics)?;
        }
        sink.checkpoint(root)?;
        sink.begin("ObjectBegin", false)?;
        sink.positional(&Token::Quoted(self.resource_name(r.id)))?;
        sink.end_directive()?;
        self.children(sink, root)?;
        sink.begin("ObjectEnd", false)?;
        sink.end_directive()?;
        sink.begin("AttributeEnd", false)?;
        sink.end_directive()?;
        Ok(())
    }

    fn declared_transform(&self, sink: &mut dyn Emission, r: &Resource) -> Result<(), Error> {
        let Some(transform) = r.declared_in.as_ref().and_then(|d| d.transform.as_ref()) else {
            return Ok(());
        };
        if transform.times != [0., 1.] {
            sink.begin("TransformTimes", false)?;
            for n in transform.times {
                sink.positional(&Token::Number(format!("{n}")))?;
            }
            sink.end_directive()?;
        }
        if transform.start == transform.end {
            self.matrix(sink, "Transform", &transform.start)?;
        } else {
            self.active_transform(sink, ActiveSamples::Start)?;
            self.matrix(sink, "Transform", &transform.start)?;
            self.active_transform(sink, ActiveSamples::End)?;
            self.matrix(sink, "Transform", &transform.end)?;
            self.active_transform(sink, ActiveSamples::All)?;
        }
        Ok(())
    }

    /// A posed camera's world-to-camera transform block: a single
    /// `Transform [inverse(pose)]` before the `Camera` directive when static;
    /// `ActiveTransform StartTime` / `EndTime` splits of both samples, then
    /// `ActiveTransform All`, when animated.
    fn emit_pose(&self, sink: &mut dyn Emission, pose: &project::CameraPose) -> Result<(), Error> {
        let inverse_of = |sample: &[f64; 16]| -> Result<[f64; 16], Error> {
            project::math::inverse(project::math::Mat4(*sample))
                .map(|matrix| matrix.0)
                .ok_or_else(|| invalid("a camera pose is not invertible"))
        };
        if pose.start == pose.end {
            self.matrix(sink, "Transform", &inverse_of(&pose.start)?)?;
        } else {
            self.active_transform(sink, ActiveSamples::Start)?;
            self.matrix(sink, "Transform", &inverse_of(&pose.start)?)?;
            self.active_transform(sink, ActiveSamples::End)?;
            self.matrix(sink, "Transform", &inverse_of(&pose.end)?)?;
            self.active_transform(sink, ActiveSamples::All)?;
        }
        Ok(())
    }

    /// The camera declaration's own transform reproduction, for cameras that
    /// carry no pose (saved projects): both samples of the evaluated state,
    /// per sample, then back to `ActiveTransform All`.
    fn declaration_transform(
        &self,
        sink: &mut dyn Emission,
        state: &project::GraphicsState,
    ) -> Result<(), Error> {
        let samples = [
            (ActiveSamples::Start, &state.start),
            (ActiveSamples::End, &state.end),
        ];
        for (active, sample) in samples {
            self.active_transform(sink, active)?;
            self.matrix(sink, "Transform", &sample.0)?;
        }
        self.active_transform(sink, ActiveSamples::All)
    }

    /// The camera-to-world matrices of a referenced camera: its `pose` when
    /// it carries one, else the inverse of the transform current at its
    /// declaration — the same two cases the evaluator applies.
    fn referenced_camera_to_world(&self, camera: NodeId) -> Result<([f64; 16], [f64; 16]), Error> {
        if let Some(project::CameraPose { ref start, ref end }) = self
            .view
            .node(camera)
            .map(|node| &node.data)
            .and_then(|data| match data {
                NodeKind::Entity { pose, .. } => *pose,
                _ => None,
            })
        {
            return Ok((*start, *end));
        }
        let state = self
            .evaluated
            .nodes
            .get(&camera)
            .ok_or_else(|| invalid("camera is missing from scene options"))?;
        let inverse_of = |sample: &project::math::Mat4| -> Result<[f64; 16], Error> {
            project::math::inverse(*sample)
                .map(|matrix| matrix.0)
                .ok_or_else(|| invalid("singular camera transform"))
        };
        Ok((inverse_of(&state.start)?, inverse_of(&state.end)?))
    }

    fn active_transform(
        &self,
        sink: &mut dyn Emission,
        active: ActiveSamples,
    ) -> Result<(), Error> {
        sink.begin("ActiveTransform", false)?;
        sink.positional(&Token::Word(
            match active {
                ActiveSamples::All => "All",
                ActiveSamples::Start => "StartTime",
                ActiveSamples::End => "EndTime",
            }
            .into(),
        ))?;
        sink.end_directive()
    }

    fn matrix(&self, sink: &mut dyn Emission, name: &str, m: &[f64; 16]) -> Result<(), Error> {
        sink.begin(name, true)?;
        for i in 0..16 {
            sink.positional(&Token::Number(format!("{}", m[(i % 4) * 4 + i / 4])))?;
        }
        sink.end_directive()
    }

    fn graphics_state(
        &self,
        sink: &mut dyn Emission,
        graphics: &project::CapturedGraphics,
    ) -> Result<(), Error> {
        if graphics.color_space != "srgb" {
            sink.begin("ColorSpace", false)?;
            sink.positional(&Token::Quoted(graphics.color_space.clone()))?;
            sink.end_directive()?;
        }
        match &graphics.material {
            MaterialBinding::Default => {}
            MaterialBinding::None => {
                sink.begin("Material", false)?;
                sink.positional(&Token::Quoted("interface".into()))?;
                sink.end_directive()?;
            }
            MaterialBinding::Resource(id) => {
                sink.begin("NamedMaterial", false)?;
                sink.positional(&Token::Quoted(self.resource_name(*id)))?;
                sink.end_directive()?;
            }
        }
        if graphics.inside_medium.is_some() || graphics.outside_medium.is_some() {
            sink.begin("MediumInterface", false)?;
            sink.positional(&Token::Quoted(
                graphics
                    .inside_medium
                    .map(|id| self.resource_name(id))
                    .unwrap_or_default(),
            ))?;
            sink.positional(&Token::Quoted(
                graphics
                    .outside_medium
                    .map(|id| self.resource_name(id))
                    .unwrap_or_default(),
            ))?;
            sink.end_directive()?;
        }
        if graphics.reverse_orientation {
            sink.begin("ReverseOrientation", false)?;
            sink.end_directive()?;
        }
        Ok(())
    }

    fn children(&self, sink: &mut dyn Emission, id: NodeId) -> Result<(), Error> {
        let node = self.view.node(id).ok_or_else(|| invalid("missing node"))?;
        let NodeKind::Scope { children, .. } = &node.data else {
            return Err(invalid("expected a scope"));
        };
        for child in children {
            self.node(sink, *child)?;
        }
        Ok(())
    }

    pub fn node(&self, sink: &mut dyn Emission, id: NodeId) -> Result<(), Error> {
        check(self.cancelled)?;
        sink.checkpoint(id)?;
        let source = self.view.node(id).ok_or_else(|| invalid("missing node"))?;
        if !source.active {
            return Ok(());
        }
        if let NodeKind::Scope {
            owned_object: Some(owner),
            ..
        } = &source.data
            && self.view.node(*owner).is_some_and(|node| !node.active)
        {
            return Ok(());
        }
        match &self
            .view
            .node(id)
            .ok_or_else(|| invalid("missing node"))?
            .data
        {
            NodeKind::Scope {
                scope,
                children,
                parent_camera,
                ..
            } => {
                let (start, end) = match scope {
                    // WorldBegin is emitted by `whole_scene` before resources.
                    ScopeKind::World => ("", ""),
                    ScopeKind::Attribute => ("AttributeBegin", "AttributeEnd"),
                    ScopeKind::Transform => ("TransformBegin", "TransformEnd"),
                    // Object scopes are emitted from their resource, not the tree.
                    ScopeKind::ObjectDefinition => ("", ""),
                };
                if !start.is_empty() {
                    sink.begin(start, false)?;
                    sink.end_directive()?;
                }
                // A camera-attached group enters with the camera's
                // camera-to-world transform, so the exported group renders
                // where the camera is without referencing it.
                if let Some(camera) = parent_camera {
                    let (start_matrix, end_matrix) = self.referenced_camera_to_world(*camera)?;
                    // The evaluator sets both samples whatever selector is
                    // active, and the children keep the entry selector.
                    let entry = self.evaluated.nodes[&id].active;
                    let last = if start_matrix == end_matrix {
                        if entry != ActiveSamples::All {
                            self.active_transform(sink, ActiveSamples::All)?;
                        }
                        self.matrix(sink, "Transform", &start_matrix)?;
                        ActiveSamples::All
                    } else {
                        self.active_transform(sink, ActiveSamples::Start)?;
                        self.matrix(sink, "Transform", &start_matrix)?;
                        self.active_transform(sink, ActiveSamples::End)?;
                        self.matrix(sink, "Transform", &end_matrix)?;
                        ActiveSamples::End
                    };
                    if last != entry {
                        self.active_transform(sink, entry)?;
                    }
                }
                for child in children {
                    self.node(sink, *child)?;
                }
                if !end.is_empty() {
                    sink.begin(end, false)?;
                    sink.end_directive()?;
                }
            }
            NodeKind::State { operation } => match operation {
                StateOperation::Transform(t) => match t {
                    TransformOperation::Identity => {
                        sink.begin("Identity", false)?;
                        sink.end_directive()?;
                    }
                    TransformOperation::Translate(v) | TransformOperation::Scale(v) => {
                        sink.begin(
                            if matches!(t, TransformOperation::Translate(_)) {
                                "Translate"
                            } else {
                                "Scale"
                            },
                            false,
                        )?;
                        for n in v {
                            sink.positional(&Token::Number(format!("{n}")))?;
                        }
                        sink.end_directive()?;
                    }
                    TransformOperation::LookAt(v) => {
                        sink.begin("LookAt", false)?;
                        for n in v {
                            sink.positional(&Token::Number(format!("{n}")))?;
                        }
                        sink.end_directive()?;
                    }
                    TransformOperation::Rotate { degrees, axis } => {
                        sink.begin("Rotate", false)?;
                        sink.positional(&Token::Number(format!("{degrees}")))?;
                        for n in axis {
                            sink.positional(&Token::Number(format!("{n}")))?;
                        }
                        sink.end_directive()?;
                    }
                    TransformOperation::Replace(m) | TransformOperation::Concat(m) => {
                        sink.begin(
                            if matches!(t, TransformOperation::Replace(_)) {
                                "Transform"
                            } else {
                                "ConcatTransform"
                            },
                            true,
                        )?;
                        for i in 0..16 {
                            sink.positional(&Token::Number(format!("{}", m[(i % 4) * 4 + i / 4])))?;
                        }
                        sink.end_directive()?;
                    }
                },
                StateOperation::ActiveTransform(active) => {
                    sink.begin("ActiveTransform", false)?;
                    sink.positional(&Token::Word(
                        match active {
                            ActiveSamples::All => "All",
                            ActiveSamples::Start => "StartTime",
                            ActiveSamples::End => "EndTime",
                        }
                        .into(),
                    ))?;
                    sink.end_directive()?;
                }
                StateOperation::TransformTimes(t) => {
                    sink.begin("TransformTimes", false)?;
                    for n in t {
                        sink.positional(&Token::Number(format!("{n}")))?;
                    }
                    sink.end_directive()?;
                }
                StateOperation::Material(binding) => match binding {
                    MaterialBinding::Resource(id) => {
                        sink.begin("NamedMaterial", false)?;
                        sink.positional(&Token::Quoted(self.resource_name(*id)))?;
                        sink.end_directive()?;
                    }
                    MaterialBinding::None => {
                        sink.begin("Material", false)?;
                        sink.positional(&Token::Quoted("interface".into()))?;
                        sink.end_directive()?;
                    }
                    MaterialBinding::Default => {
                        return Err(unsupported("explicit reset to implicit default material"));
                    }
                },
                StateOperation::Medium { inside, outside } => {
                    sink.begin("MediumInterface", false)?;
                    sink.positional(&Token::Quoted(
                        inside.map(|id| self.resource_name(id)).unwrap_or_default(),
                    ))?;
                    sink.positional(&Token::Quoted(
                        outside.map(|id| self.resource_name(id)).unwrap_or_default(),
                    ))?;
                    sink.end_directive()?;
                }
                StateOperation::ReverseOrientation => {
                    sink.begin("ReverseOrientation", false)?;
                    sink.end_directive()?;
                }
                StateOperation::ColorSpace(s) => {
                    sink.begin("ColorSpace", false)?;
                    sink.positional(&Token::Quoted(s.clone()))?;
                    sink.end_directive()?;
                }
                StateOperation::DefineCoordinates(s) => {
                    sink.begin("CoordinateSystem", false)?;
                    sink.positional(&Token::Quoted(s.clone()))?;
                    sink.end_directive()?;
                }
                StateOperation::UseCoordinates(s) => {
                    sink.begin("CoordSysTransform", false)?;
                    sink.positional(&Token::Quoted(s.clone()))?;
                    sink.end_directive()?;
                }
            },
            NodeKind::Instance { definition } => {
                sink.begin("ObjectInstance", false)?;
                sink.positional(&Token::Quoted(self.resource_name(*definition)))?;
                sink.end_directive()?;
            }
            NodeKind::Entity {
                entity,
                implementation,
                parameters,
                pose,
                ..
            } => {
                // A posed camera writes its inverse pose ahead of its
                // directive wherever it sits in Options.
                if matches!(entity, EntityKind::Camera)
                    && let Some(pose) = pose.as_ref()
                {
                    self.emit_pose(sink, pose)?;
                }
                sink.begin(
                    match entity {
                        EntityKind::Shape => "Shape",
                        EntityKind::Camera => "Camera",
                        EntityKind::Light => "LightSource",
                        EntityKind::AreaLight => "AreaLightSource",
                    },
                    false,
                )?;
                sink.positional(&Token::Quoted(implementation.clone()))?;
                self.parameters(sink, parameters, &self.evaluated.nodes[&id].color_space)?;
                sink.end_directive()?;
            }
            NodeKind::Configuration {
                configuration,
                implementation,
                parameters,
            } => {
                sink.begin(
                    match configuration {
                        ConfigurationKind::Film => "Film",
                        ConfigurationKind::Sampler => "Sampler",
                        ConfigurationKind::Integrator => "Integrator",
                        ConfigurationKind::PixelFilter => "PixelFilter",
                        ConfigurationKind::Accelerator => "Accelerator",
                    },
                    false,
                )?;
                sink.positional(&Token::Quoted(implementation.clone()))?;
                if *configuration == ConfigurationKind::Film {
                    let params = parameters
                        .iter()
                        .filter(|p| p.name != "filename")
                        .cloned()
                        .collect::<Vec<_>>();
                    self.parameters(sink, &params, &self.evaluated.nodes[&id].color_space)?;
                    sink.film_filename()?;
                } else {
                    self.parameters(sink, parameters, &self.evaluated.nodes[&id].color_space)?;
                }
                sink.end_directive()?;
            }
            NodeKind::Option { name, value } => {
                sink.begin("Option", false)?;
                sink.positional(&Token::Quoted(name.clone()))?;
                sink.positional(&match value {
                    OptionValue::Bool(value) => Token::Word(value.to_string()),
                    OptionValue::Integer(value) => Token::Number(value.to_string()),
                    OptionValue::Float(value) => Token::Number(scalar(value)),
                    OptionValue::String(value) => Token::Quoted(value.clone()),
                })?;
                sink.end_directive()?;
            }
        }
        Ok(())
    }

    fn parameters(
        &self,
        sink: &mut dyn Emission,
        parameters: &[project::Parameter],
        color_space: &str,
    ) -> Result<(), Error> {
        for p in parameters {
            check(self.cancelled)?;
            if matches!(&p.color_space, project::ColorSpaceBinding::Explicit(s) if s != color_space)
            {
                return Err(unsupported("per-parameter color space override"));
            }
            sink.begin_parameter(&p.value_type, &p.name)?;
            match &p.values {
                ParameterValues::Floats(v) => {
                    for n in v {
                        sink.parameter_value(&Token::Number(scalar(n)))?;
                    }
                }
                ParameterValues::Integers(v) => {
                    for n in v {
                        sink.parameter_value(&Token::Number(format!("{n}")))?;
                    }
                }
                ParameterValues::Booleans(v) => {
                    for n in v {
                        sink.parameter_value(&Token::Word(format!("{n}")))?;
                    }
                }
                ParameterValues::Strings(v) | ParameterValues::BuiltinSpectra(v) => {
                    for s in v {
                        sink.parameter_value(&Token::Quoted(s.clone()))?;
                    }
                }
                ParameterValues::Resources {
                    kind: ResourceKind::SpectrumData,
                    ids,
                } => {
                    for id in ids {
                        if sink.expand_spectrum_samples() {
                            let resource = self
                                .view
                                .resource(*id)
                                .ok_or_else(|| invalid("missing spectrum data resource"))?;
                            let (wavelengths, values) = spectrum_samples(resource, |asset| {
                                Ok(match self.snapshot {
                                    Some(snapshot) => snapshot.asset_path(self.store, asset)?,
                                    None => self.store.asset_path(asset)?,
                                })
                            })?;
                            // The preview transfer decimates long curves. The text
                            // export's sink never reaches this code.
                            let curve = decimated_preview_curve(
                                wavelengths.into_iter().zip(values).collect(),
                            );
                            for (wavelength, value) in curve {
                                sink.parameter_value(&Token::Number(wavelength.to_string()))?;
                                sink.parameter_value(&Token::Number(value.to_string()))?;
                            }
                        } else {
                            let path = self
                                .spectra
                                .get(id)
                                .cloned()
                                .unwrap_or_else(|| self.resource_name(*id));
                            sink.parameter_value(&Token::Quoted(path))?;
                        }
                    }
                }
                ParameterValues::Resources { ids, .. } => {
                    for id in ids {
                        sink.parameter_value(&Token::Quoted(self.resource_name(*id)))?;
                    }
                }
                ParameterValues::Assets(ids) => {
                    for id in ids {
                        sink.parameter_value(&Token::Quoted(self.assets[id].clone()))?;
                    }
                }
                ParameterValues::Array {
                    asset,
                    encoding,
                    count,
                } => {
                    if !self.view.document.assets.contains_key(asset) {
                        return Err(invalid("missing array asset"));
                    }
                    let mut input =
                        std::io::BufReader::new(std::fs::File::open(match self.snapshot {
                            Some(snapshot) => snapshot.asset_path(self.store, *asset)?,
                            None => self.store.asset_path(*asset)?,
                        })?);
                    for index in 0..*count {
                        if index % 4096 == 0 {
                            check(self.cancelled)?;
                        }
                        let mut bytes = [0; 8];
                        match encoding {
                            project::ArrayEncoding::F64Le => {
                                input.read_exact(&mut bytes)?;
                                let n = f64::from_le_bytes(bytes);
                                sink.parameter_value(&Token::Number(if n.is_nan() {
                                    "nan".into()
                                } else {
                                    n.to_string()
                                }))?;
                            }
                            project::ArrayEncoding::I64Le => {
                                input.read_exact(&mut bytes)?;
                                sink.parameter_value(&Token::Number(
                                    i64::from_le_bytes(bytes).to_string(),
                                ))?;
                            }
                            project::ArrayEncoding::BoolU8 => {
                                input.read_exact(&mut bytes[..1])?;
                                if bytes[0] > 1 {
                                    return Err(invalid("invalid bool array byte"));
                                }
                                sink.parameter_value(&Token::Word((bytes[0] == 1).to_string()))?;
                            }
                        }
                    }
                }
            }
            sink.end_parameter()?;
        }
        Ok(())
    }
}

/// The scene.pbrt text consumer. Byte-for-byte the previous Writer:
/// whitespace quirks live HERE and nowhere else.
pub(crate) struct TextSink<'a, W: Write> {
    pub(crate) file: crate::export::LineWriter<W>,
    pub(crate) node_sources: &'a mut BTreeMap<u64, NodeId>,
    bracket: bool,
}

impl<'a, W: Write> TextSink<'a, W> {
    pub(crate) fn new(
        file: crate::export::LineWriter<W>,
        node_sources: &'a mut BTreeMap<u64, NodeId>,
    ) -> Self {
        Self {
            file,
            node_sources,
            bracket: false,
        }
    }
}

impl<W: Write> Emission for TextSink<'_, W> {
    fn checkpoint(&mut self, node: NodeId) -> Result<(), Error> {
        self.node_sources.insert(self.file.line, node);
        Ok(())
    }
    fn begin(&mut self, name: &str, bracket: bool) -> Result<(), Error> {
        self.bracket = bracket;
        if bracket {
            write!(self.file, "{name} [")?;
        } else {
            write!(self.file, "{name}")?;
        }
        Ok(())
    }
    fn positional(&mut self, token: &Token) -> Result<(), Error> {
        write!(self.file, " {}", token.lexeme())?;
        Ok(())
    }
    fn builtin_type(&mut self, implementation: &str) -> Result<(), Error> {
        write!(self.file, " \"string type\" [{}]", quoted(implementation))?;
        Ok(())
    }
    fn begin_parameter(&mut self, value_type: &str, name: &str) -> Result<(), Error> {
        write!(self.file, " {} [", quoted(&format!("{value_type} {name}")))?;
        Ok(())
    }
    fn parameter_value(&mut self, token: &Token) -> Result<(), Error> {
        write!(self.file, " {}", token.lexeme())?;
        Ok(())
    }
    fn end_parameter(&mut self) -> Result<(), Error> {
        write!(self.file, " ]")?;
        Ok(())
    }
    fn film_filename(&mut self) -> Result<(), Error> {
        write!(self.file, " \"string filename\" [\"render.exr\"]")?;
        Ok(())
    }
    fn end_directive(&mut self) -> Result<(), Error> {
        if self.bracket {
            write!(self.file, " ]")?;
        }
        writeln!(self.file)?;
        self.bracket = false;
        Ok(())
    }
}

/// The program consumer: shares the emitter with the text path so the two
/// outputs cannot drift. One `Emission` event = one scene.pbrt token
/// sequence; every value is classified by the real lexer.
pub(crate) struct ProgramSink {
    staged: Option<StagedDirective>,
    pending_origin: Option<NodeId>,
    program: scene_parse::PreviewProgram,
    origins: BTreeMap<u32, protocol::EntityId>,
}

/// Directive under construction. `in_parameter` is the open parameter, if
/// any; positional and parameter values accumulate until `end_directive`.
struct StagedDirective {
    name: String,
    bracket: bool,
    positional: Vec<scene_parse::Value>,
    positional_tokens: Vec<String>,
    parameters: Vec<scene_parse::Parameter>,
    in_parameter: Option<scene_parse::Parameter>,
}

impl ProgramSink {
    pub(crate) fn new() -> Self {
        Self {
            staged: None,
            pending_origin: None,
            program: scene_parse::PreviewProgram::default(),
            origins: BTreeMap::new(),
        }
    }

    pub(crate) fn finish(mut self) -> scene_parse::PreviewProgram {
        assert!(self.staged.is_none(), "program sink ended mid-directive");
        self.program.directives.shrink_to_fit();
        self.program.origins = self.origins;
        self.program
    }

    fn staged_mut(&mut self) -> Result<&mut StagedDirective, Error> {
        self.staged
            .as_mut()
            .ok_or_else(|| invalid("emitter produced a token outside a directive".to_string()))
    }
}

impl Default for ProgramSink {
    fn default() -> Self {
        Self::new()
    }
}

impl Emission for ProgramSink {
    fn checkpoint(&mut self, node: NodeId) -> Result<(), Error> {
        // The node that most recently started owns every directive emitted
        // until the next checkpoint (mirrors TextSink's line table).
        self.pending_origin = Some(node);
        Ok(())
    }
    fn begin(&mut self, name: &str, bracket: bool) -> Result<(), Error> {
        let origin = self.pending_origin.take();
        self.staged = Some(StagedDirective {
            name: name.to_string(),
            bracket,
            positional: Vec::new(),
            positional_tokens: Vec::new(),
            parameters: Vec::new(),
            in_parameter: None,
        });
        if let Some(node) = origin {
            self.origins
                .insert(self.next_line(), EntityId::new(node.to_string()));
        }
        Ok(())
    }
    fn positional(&mut self, token: &Token) -> Result<(), Error> {
        let lexeme = token.lexeme();
        let staged = self.staged_mut()?;
        staged
            .positional
            .push(scene_parse::Value::classify(&lexeme));
        staged.positional_tokens.push(lexeme);
        Ok(())
    }
    fn builtin_type(&mut self, implementation: &str) -> Result<(), Error> {
        // Same semantics the text path prints: a leading `string type`
        // parameter (packed there, plain here).
        self.begin_parameter("string", "type")?;
        self.parameter_value(&Token::Quoted(implementation.to_string()))?;
        self.end_parameter()
    }
    fn begin_parameter(&mut self, value_type: &str, name: &str) -> Result<(), Error> {
        let staged = self.staged_mut()?;
        staged.in_parameter = Some(scene_parse::Parameter {
            value_type: value_type.to_string(),
            name: name.to_string(),
            values: Vec::new(),
            value_tokens: Vec::new(),
            line: 0,
            column: 0,
        });
        Ok(())
    }
    fn parameter_value(&mut self, token: &Token) -> Result<(), Error> {
        let lexeme = token.lexeme();
        let staged = self.staged_mut()?;
        let parameter = staged
            .in_parameter
            .as_mut()
            .ok_or_else(|| invalid("emitter produced a value outside a parameter".to_string()))?;
        parameter.values.push(scene_parse::Value::classify(&lexeme));
        parameter.value_tokens.push(lexeme);
        Ok(())
    }
    fn end_parameter(&mut self) -> Result<(), Error> {
        let staged = self.staged_mut()?;
        if let Some(parameter) = staged.in_parameter.take() {
            staged.parameters.push(parameter);
        }
        Ok(())
    }
    fn film_filename(&mut self) -> Result<(), Error> {
        self.begin_parameter("string", "filename")?;
        self.parameter_value(&Token::Quoted("render.exr".to_string()))?;
        self.end_parameter()
    }
    /// The program resolves SpectrumData references itself.
    fn expand_spectrum_samples(&self) -> bool {
        true
    }
    /// The preview never draws media; the program keeps the medium's
    /// name and type without expanding its bulk arrays.
    fn medium_parameters(&self) -> bool {
        false
    }
    fn end_directive(&mut self) -> Result<(), Error> {
        let staged = self
            .staged
            .take()
            .ok_or_else(|| invalid("emitter ended a directive it never began".to_string()))?;
        if staged.in_parameter.is_some() {
            return Err(invalid(
                "emitter ended a directive inside an open parameter".to_string(),
            ));
        }
        // `bracket` is text syntax for matrix transforms; the positional
        // values are already complete, so the program keeps the directive.
        let _ = staged.bracket;
        // The 1-based ordinal doubles as the directive's `line`, matching
        // the origin table keys checked out at checkpoints.
        let line = self.next_line();
        self.program.directives.push(scene_parse::Directive {
            offset: 0,
            length: 0,
            positional_tokens: staged.positional_tokens,
            name: staged.name,
            path: scene_parse::PREVIEW_PATH.to_string(),
            line,
            column: 0,
            positional: staged.positional,
            parameters: staged.parameters,
        });
        Ok(())
    }
}

impl ProgramSink {
    fn next_line(&self) -> u32 {
        self.program.directives.len() as u32 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Option` stores a typed value and the emitter prints its
    /// canonical lexical form.
    #[test]
    fn option_values_are_emitted_from_their_typed_value() {
        let temp = tempfile::tempdir().unwrap();
        let next_project = std::cell::Cell::new(0);
        let emit = |value: OptionValue| -> Result<scene_parse::PreviewProgram, Error> {
            let mut document = project::ProjectDocument::new("Option");
            let node = project::Node::new(
                "Option",
                NodeKind::Option {
                    name: "seed".into(),
                    value,
                },
            );
            let id = node.id;
            let scene_id = document.default_scene.unwrap();
            let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
            scene.options.push(id);
            scene.nodes.insert(id, node);
            let view = document.default_scene_view().unwrap();
            let evaluated = project::evaluate(&view).unwrap();
            let names = resource_names(&document);
            let assets = BTreeMap::new();
            let spectra = BTreeMap::new();
            let project_path = temp.path().join(format!("project-{}", next_project.get()));
            next_project.set(next_project.get() + 1);
            let store = project::ProjectStore::Folder(
                project::FolderStore::create(
                    &project_path,
                    document.id,
                    &document.title,
                    project::Revision::initial(),
                )
                .unwrap(),
            );
            let emitter = Emitter {
                snapshot: None,
                view,
                evaluated: &evaluated,
                store: &store,
                assets: &assets,
                names: &names,
                spectra: &spectra,
                cancelled: &|| false,
            };
            let mut sink = ProgramSink::new();
            emitter.whole_scene(&mut sink)?;
            Ok(sink.finish())
        };

        for (value, expected) in [
            (OptionValue::Integer(42), "42"),
            (OptionValue::Bool(true), "true"),
            (OptionValue::String("camera".into()), "\"camera\""),
        ] {
            let program = emit(value).expect("typed Option");
            let option = program
                .directives
                .iter()
                .find(|directive| directive.name == "Option")
                .expect("Option directive");
            assert_eq!(option.positional_tokens, vec!["\"seed\"", expected]);
        }
    }
}
