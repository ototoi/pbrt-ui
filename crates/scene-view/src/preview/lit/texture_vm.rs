//! Typed, bounded texture graph compilation for the Lit preview.
//!
//! CPU compilation and WGSL execution share a stable 32-byte instruction
//! layout. GPU upload and image binding stay in the renderer.
use protocol::geometry::{PreviewParameter, TextureRecord};
use std::collections::BTreeSet;

pub(crate) const MAX_INSTRUCTIONS_PER_ROOT: usize = 256;
pub(crate) const MAX_REFERENCE_DEPTH: usize = 32;

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TextureInstruction {
    pub op: u32,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub value: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ValueType {
    Float,
    Spectrum,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextureProgram {
    pub value_type: ValueType,
    pub instructions: Vec<TextureInstruction>,
    /// Image names used by this root, in deterministic first-use order.
    pub images: Vec<String>,
    /// Point-to-texture rows `OP_MAP.b` indexes, local to this program.
    pub mappings: Vec<MappingRows>,
    pub diagnostics: Vec<String>,
}

pub(crate) const OP_FLOAT: u32 = 0;
pub(crate) const OP_SPECTRUM: u32 = 1;
pub(crate) const OP_IMAGE: u32 = 2;
pub(crate) const OP_SCALE: u32 = 3;
pub(crate) const OP_MIX: u32 = 4;
pub(crate) const OP_BILERP: u32 = 5;
pub(crate) const OP_CHECKERBOARD: u32 = 6;
pub(crate) const OP_DOTS: u32 = 7;
pub(crate) const OP_DIRECTIONMIX: u32 = 8;
pub(crate) const OP_UV: u32 = 9;
/// Maps the pixel's position for the next instruction, without touching the
/// stack: `a` is a `MAP_*` kind and `b` a row table index.
pub(crate) const OP_MAP: u32 = 10;
pub(crate) const OP_CHECKERBOARD3D: u32 = 11;
pub(crate) const OP_FBM: u32 = 12;
pub(crate) const OP_WRINKLED: u32 = 13;
pub(crate) const OP_WINDY: u32 = 14;
pub(crate) const OP_MARBLE: u32 = 15;

pub(crate) const MAP_PLANAR: u32 = 0;
pub(crate) const MAP_SPHERICAL: u32 = 1;
pub(crate) const MAP_CYLINDRICAL: u32 = 2;
pub(crate) const MAP_POINT: u32 = 3;

/// Three rows of a world-to-texture matrix. They stay in f64 until a frame's
/// origin is known, so that far scenes keep their pattern.
pub(crate) type MappingRows = [[f64; 4]; 3];
/// Bounds the shader's octave loops; pbrt's sums stop changing long before.
const MAX_OCTAVES: f64 = 32.;

/// r4's `NOISEPERM` (textures/noise.rs); the shader indexes it doubled.
#[rustfmt::skip]
pub(crate) const NOISE_PERM: [u32; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225,
    140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148,
    247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32,
    57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122,
    60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54,
    65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169,
    200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64,
    52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
    207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213,
    119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9,
    129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104,
    218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241,
    81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157,
    184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93,
    222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
];

/// A program's place in the instruction buffer, as `program_value` reads it.
/// Zero means none: a program always has an instruction.
pub(crate) fn pack(start: usize, count: usize) -> u32 {
    debug_assert!((1..=MAX_INSTRUCTIONS_PER_ROOT).contains(&count));
    (start as u32) << 9 | count as u32
}

pub(crate) fn compile(
    root: &str,
    value_type: ValueType,
    records: &[TextureRecord],
) -> TextureProgram {
    let mut compiler = Compiler {
        records,
        instructions: Vec::new(),
        images: Vec::new(),
        mappings: Vec::new(),
        active: BTreeSet::new(),
        diagnostics: Vec::new(),
        budget_exceeded: false,
        invalid_graph: false,
    };
    compiler.compile_ref(root, value_type, 0);
    if compiler.budget_exceeded || compiler.invalid_graph {
        if compiler.budget_exceeded {
            compiler.diagnostics.push(format!(
                "texture program exceeds {MAX_INSTRUCTIONS_PER_ROOT} instructions"
            ));
        }
        compiler.instructions.clear();
        compiler.budget_exceeded = false;
        compiler.push_constant(value_type, [0.5, 0.5, 0.5, 0.5]);
        compiler.images.clear();
        compiler.mappings.clear();
    } else if compiler.instructions.len() > MAX_INSTRUCTIONS_PER_ROOT {
        compiler.diagnostics.push(format!(
            "texture program exceeds {MAX_INSTRUCTIONS_PER_ROOT} instructions"
        ));
        compiler.instructions.clear();
        compiler.push_constant(value_type, [0.5, 0.5, 0.5, 0.5]);
        compiler.images.clear();
        compiler.mappings.clear();
    }
    if !valid_stack(&compiler.instructions) {
        compiler
            .diagnostics
            .push("texture program stack is invalid".into());
        compiler.instructions.clear();
        compiler.push_constant(value_type, [0.5, 0.5, 0.5, 0.5]);
        compiler.images.clear();
        compiler.mappings.clear();
    }
    TextureProgram {
        value_type,
        instructions: compiler.instructions,
        images: compiler.images,
        mappings: compiler.mappings,
        diagnostics: compiler.diagnostics,
    }
}

struct Compiler<'a> {
    records: &'a [TextureRecord],
    instructions: Vec<TextureInstruction>,
    images: Vec<String>,
    mappings: Vec<MappingRows>,
    active: BTreeSet<(String, ValueType)>,
    diagnostics: Vec<String>,
    budget_exceeded: bool,
    invalid_graph: bool,
}

impl Compiler<'_> {
    fn compile_ref(&mut self, name: &str, expected: ValueType, depth: usize) {
        if self.budget_exceeded {
            return;
        }
        if depth >= MAX_REFERENCE_DEPTH {
            self.invalid_fallback(
                expected,
                format!("texture reference depth exceeds {MAX_REFERENCE_DEPTH}"),
            );
            return;
        }
        let key = (name.to_owned(), expected);
        if !self.active.insert(key.clone()) {
            self.invalid_fallback(expected, format!("texture cycle at {name}"));
            return;
        }
        let expected_name = match expected {
            ValueType::Float => "float",
            ValueType::Spectrum => "spectrum",
        };
        let Some(record) = self.records.iter().find(|record| {
            record.name == name && record.value_type.eq_ignore_ascii_case(expected_name)
        }) else {
            self.invalid_fallback(expected, format!("missing {expected_name} texture {name}"));
            self.active.remove(&key);
            return;
        };
        match record.implementation.to_ascii_lowercase().as_str() {
            "constant" => {
                self.compile_constant(name, expected, parameter(&record.parameters, "value"));
            }
            "imagemap" => {
                let mapping = self.mapping_2d(record);
                if let Err(message) = &mapping {
                    self.fallback(expected, format!("imagemap {name} {message}"));
                } else if parameter(&record.parameters, "filename")
                    .and_then(|p| p.texts.first())
                    .is_none_or(|name| name.is_empty())
                {
                    self.fallback(expected, format!("imagemap {name} has no filename"));
                } else {
                    // Typed identity is required because pbrt float imagemaps
                    // read a scalar while spectrum imagemaps read RGB.
                    let image_key = format!("texture/{expected_name}/{name}");
                    let slot = self
                        .images
                        .iter()
                        .position(|image| image == &image_key)
                        .unwrap_or_else(|| {
                            self.images.push(image_key);
                            self.images.len() - 1
                        });
                    let wrap = text_parameter(&record.parameters, "wrap").unwrap_or("repeat");
                    let wrap = match wrap.to_ascii_lowercase().as_str() {
                        "clamp" => 1,
                        "black" => 2,
                        _ => 0,
                    };
                    // Float imagemaps read a scalar: the binding path stores
                    // them as one channel and the shader widens R to RGB
                    // (r4's `node.kind == 0u` rule); bit 8 of `b` carries
                    // that, the wrap keeps the low byte.
                    let wrap = wrap | ((expected == ValueType::Float) as u32) << 8;
                    let scale = parameter(&record.parameters, "scale")
                        .and_then(|p| p.floats.first())
                        .copied()
                        .unwrap_or(1.0) as f32;
                    let value = self.push_mapping(mapping.unwrap_or(Mapping2D::Uv(UV_IDENTITY)));
                    self.push(TextureInstruction {
                        op: OP_IMAGE,
                        a: slot as u32,
                        b: wrap,
                        c: scale.to_bits(),
                        value,
                    });
                }
            }
            "scale" => {
                self.compile_scale(record, name, expected, depth);
            }
            "mix" => {
                self.compile_mix(record, name, expected, depth);
            }
            "checkerboard" => self.compile_checkerboard(record, name, expected, depth),
            "dots" => {
                if record
                    .parameters
                    .iter()
                    .any(|p| p.matches("tex1") || p.matches("tex2"))
                {
                    self.diagnostics.push(format!(
                        "dots texture {name}: tex1 and tex2 are ignored; pbrt reads inside and outside"
                    ));
                }
                // As in pbrt-v4 and r4, `inside` shows outside the dots and
                // `outside` within them.
                self.compile_two_inputs(
                    record,
                    name,
                    expected,
                    depth,
                    OP_DOTS,
                    0,
                    [("inside", 1.), ("outside", 0.)],
                )
            }
            "bilerp" => self.compile_bilerp(record, name, expected, depth),
            "directionmix" => self.compile_directionmix(record, name, expected, depth),
            "uv" if expected == ValueType::Spectrum => match self.mapping_2d(record) {
                Ok(mapping) => {
                    let value = self.push_mapping(mapping);
                    self.push(TextureInstruction {
                        op: OP_UV,
                        a: 0,
                        b: 0,
                        c: 0,
                        value,
                    });
                }
                Err(message) => self.fallback(expected, format!("uv texture {name} {message}")),
            },
            "fbm" | "wrinkled" | "windy" => self.compile_noise(record, name, expected),
            "marble" if expected == ValueType::Spectrum => self.compile_marble(record, name),
            other => self.fallback(expected, format!("unsupported texture {other}")),
        }
        self.active.remove(&key);
    }

    fn push_constant(&mut self, value_type: ValueType, value: [f32; 4]) {
        self.push(TextureInstruction {
            op: match value_type {
                ValueType::Float => OP_FLOAT,
                ValueType::Spectrum => OP_SPECTRUM,
            },
            a: 0,
            b: 0,
            c: 0,
            value,
        });
    }

    fn compile_constant(
        &mut self,
        name: &str,
        expected: ValueType,
        value: Option<&PreviewParameter>,
    ) {
        let Some(value) = value else {
            // pbrt-v4 ConstantTexture defaults both float and spectrum values
            // to one.
            self.push_constant(expected, [1.0; 4]);
            return;
        };

        let literal = match expected {
            ValueType::Float
                if value.value_type.eq_ignore_ascii_case("float") && value.floats.len() == 1 =>
            {
                Some([value.floats[0] as f32; 4])
            }
            ValueType::Spectrum
                if value.value_type.eq_ignore_ascii_case("float") && value.floats.len() == 1 =>
            {
                let scalar = value.floats[0] as f32;
                Some([scalar, scalar, scalar, 1.0])
            }
            ValueType::Spectrum
                if ["rgb", "color"]
                    .iter()
                    .any(|kind| value.value_type.eq_ignore_ascii_case(kind))
                    && value.floats.len() == 3 =>
            {
                Some([
                    value.floats[0] as f32,
                    value.floats[1] as f32,
                    value.floats[2] as f32,
                    1.0,
                ])
            }
            _ => None,
        };
        if let Some(literal) = literal {
            self.push_constant(expected, literal);
        } else {
            self.invalid_fallback(
                expected,
                format!(
                    "constant texture {name} value {:?} is unsupported; expected a float literal or RGB literal",
                    value.value_type
                ),
            );
        }
    }

    fn compile_scale(
        &mut self,
        record: &TextureRecord,
        name: &str,
        expected: ValueType,
        depth: usize,
    ) {
        // pbrt-v4 uses `tex` and a float `scale`; r4 also accepts the v3
        // `tex1`/`tex2` names as aliases. Preserve that alias with a visible
        // diagnostic because v3 `tex2` may be a spectrum rather than v4's
        // float multiplier.
        let v4_tex = parameter(&record.parameters, "tex");
        let legacy_tex = parameter(&record.parameters, "tex1");
        let legacy_tex2 = parameter(&record.parameters, "tex2");
        let legacy = v4_tex.is_none() && legacy_tex.is_some();
        if legacy {
            self.diagnostics
                .push(format!("scale texture {name} uses legacy tex1/tex2 inputs"));
        }
        if let Some(primary) = v4_tex.or(legacy_tex) {
            self.compile_operand(Some(primary), expected, None, name, "tex", depth + 1);
        } else {
            // pbrt-v4 defaults the input texture to one.
            self.push_constant(expected, [1.0; 4]);
        }

        if legacy {
            self.compile_operand(
                legacy_tex2,
                expected,
                Some([1.0; 4]),
                name,
                "tex2",
                depth + 1,
            );
        } else if let Some(legacy_tex2) = legacy_tex2 {
            // r4 accepts `tex2` as a float-scale alias; retain it for older
            // scenes, but surface that it is not the pbrt-v4 spelling.
            self.diagnostics.push(format!(
                "scale texture {name} uses legacy tex2 as a scale alias"
            ));
            self.compile_operand(
                Some(legacy_tex2),
                ValueType::Float,
                Some([1.0; 4]),
                name,
                "tex2",
                depth + 1,
            );
        } else {
            self.compile_operand(
                parameter(&record.parameters, "scale"),
                ValueType::Float,
                Some([1.0; 4]),
                name,
                "scale",
                depth + 1,
            );
        }
        self.push(TextureInstruction {
            op: OP_SCALE,
            a: 0,
            b: 0,
            c: 0,
            value: [0.0; 4],
        });
    }

    fn compile_mix(
        &mut self,
        record: &TextureRecord,
        name: &str,
        expected: ValueType,
        depth: usize,
    ) {
        let (default_tex1, default_tex2) = match expected {
            // r4 MixSpectrumTexture::create defaults tex1 to one and tex2 to
            // zero. Float MixTexture uses the opposite defaults.
            ValueType::Float => ([0.0; 4], [1.0; 4]),
            ValueType::Spectrum => ([1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
        };
        self.compile_operand(
            parameter(&record.parameters, "tex1"),
            expected,
            Some(default_tex1),
            name,
            "tex1",
            depth + 1,
        );
        self.compile_operand(
            parameter(&record.parameters, "tex2"),
            expected,
            Some(default_tex2),
            name,
            "tex2",
            depth + 1,
        );
        self.compile_operand(
            parameter(&record.parameters, "amount"),
            ValueType::Float,
            Some([0.5; 4]),
            name,
            "amount",
            depth + 1,
        );
        self.push(TextureInstruction {
            op: OP_MIX,
            a: 0,
            b: 0,
            c: 0,
            value: [0.0; 4],
        });
    }

    /// `tex1`, `tex2` then `op`, whose operand is the UV mapping. Both inputs
    /// are evaluated unconditionally; the instruction selects between them.
    #[allow(clippy::too_many_arguments)]
    fn compile_two_inputs(
        &mut self,
        record: &TextureRecord,
        name: &str,
        expected: ValueType,
        depth: usize,
        op: u32,
        a: u32,
        inputs: [(&str, f32); 2],
    ) {
        let mapping = match self.mapping_2d(record) {
            Ok(mapping) => mapping,
            Err(message) => {
                self.fallback(expected, format!("texture {name} {message}"));
                return;
            }
        };
        for (input, default) in inputs {
            self.compile_operand(
                parameter(&record.parameters, input),
                expected,
                Some(literal_default(expected, default)),
                name,
                input,
                depth + 1,
            );
        }
        let value = self.push_mapping(mapping);
        self.push(TextureInstruction {
            op,
            a,
            b: 0,
            c: 0,
            value,
        });
    }

    /// The instruction's UV value, after the map instruction any other mapping
    /// needs. Called just before the instruction that consumes the mapping, so
    /// its inputs' own mappings cannot come between them.
    fn push_mapping(&mut self, mapping: Mapping2D) -> [f32; 4] {
        match mapping {
            Mapping2D::Uv(value) => value,
            Mapping2D::Mapped(kind, rows) => {
                self.push_map(kind, rows);
                UV_IDENTITY
            }
        }
    }

    fn push_map(&mut self, kind: u32, rows: MappingRows) {
        let slot = self
            .mappings
            .iter()
            .position(|known| *known == rows)
            .unwrap_or_else(|| {
                self.mappings.push(rows);
                self.mappings.len() - 1
            });
        self.push(TextureInstruction {
            op: OP_MAP,
            a: kind,
            b: slot as u32,
            c: 0,
            value: [0.; 4],
        });
    }

    /// r4's `TextureMapping2D::create`.
    fn mapping_2d(&self, record: &TextureRecord) -> Result<Mapping2D, String> {
        let p = &record.parameters;
        let float = |name: &str, default: f64| {
            parameter(p, name)
                .and_then(|v| v.floats.first())
                .copied()
                .unwrap_or(default)
        };
        let mapping = text_parameter(p, "mapping")
            .unwrap_or("uv")
            .to_ascii_lowercase();
        if mapping == "uv" {
            return Ok(Mapping2D::Uv(
                [
                    float("uscale", 1.),
                    float("vscale", 1.),
                    float("udelta", 0.),
                    float("vdelta", 0.),
                ]
                .map(|v| v as f32),
            ));
        }
        let rows = texture_from_world(record)?;
        match mapping.as_str() {
            "spherical" => Ok(Mapping2D::Mapped(MAP_SPHERICAL, rows)),
            "cylindrical" => Ok(Mapping2D::Mapped(MAP_CYLINDRICAL, rows)),
            "planar" => {
                let vector = |name: &str, default: [f64; 3]| match parameter(p, name)
                    .map(|v| v.floats.as_slice())
                {
                    Some(&[x, y, z]) => [x, y, z],
                    _ => default,
                };
                let project = |axis: [f64; 3], delta: f64| -> [f64; 4] {
                    let mut row: [f64; 4] =
                        std::array::from_fn(|c| (0..3).map(|r| axis[r] * rows[r][c]).sum());
                    row[3] += delta;
                    row
                };
                Ok(Mapping2D::Mapped(
                    MAP_PLANAR,
                    [
                        project(vector("v1", [1., 0., 0.]), float("udelta", 0.)),
                        project(vector("v2", [0., 1., 0.]), float("vdelta", 0.)),
                        [0.; 4],
                    ],
                ))
            }
            other => Err(format!("mapping {other} is unsupported; using 0.5")),
        }
    }

    fn compile_noise(&mut self, record: &TextureRecord, name: &str, expected: ValueType) {
        let rows = match texture_from_world(record) {
            Ok(rows) => rows,
            Err(message) => {
                self.fallback(expected, format!("texture {name} {message}"));
                return;
            }
        };
        let p = &record.parameters;
        let (op, omega) = match record.implementation.to_ascii_lowercase().as_str() {
            "fbm" => (OP_FBM, float_of(p, "roughness", 0.5)),
            "wrinkled" => (OP_WRINKLED, float_of(p, "roughness", 0.5)),
            _ => (OP_WINDY, 0.),
        };
        self.push_map(MAP_POINT, rows);
        self.push(TextureInstruction {
            op,
            a: float_of(p, "octaves", 8.).clamp(0., MAX_OCTAVES) as u32,
            b: 0,
            c: 0,
            value: [omega as f32, 0., 0., 0.],
        });
    }

    fn compile_marble(&mut self, record: &TextureRecord, name: &str) {
        let rows = match texture_from_world(record) {
            Ok(rows) => rows,
            Err(message) => {
                self.fallback(ValueType::Spectrum, format!("marble {name} {message}"));
                return;
            }
        };
        let p = &record.parameters;
        self.push_map(MAP_POINT, rows);
        self.push(TextureInstruction {
            op: OP_MARBLE,
            a: float_of(p, "octaves", 8.).clamp(0., MAX_OCTAVES) as u32,
            b: 0,
            c: 0,
            value: [
                float_of(p, "roughness", 0.5) as f32,
                float_of(p, "scale", 1.) as f32,
                float_of(p, "variation", 0.2) as f32,
                0.,
            ],
        });
    }

    fn compile_checkerboard(
        &mut self,
        record: &TextureRecord,
        name: &str,
        expected: ValueType,
        depth: usize,
    ) {
        let dimension = parameter(&record.parameters, "dimension")
            .and_then(|p| p.floats.first())
            .copied()
            .unwrap_or(2.);
        if dimension == 3. {
            self.compile_checkerboard_3d(record, name, expected, depth);
            return;
        }
        if dimension != 2. {
            self.fallback(
                expected,
                format!("checkerboard {name} dimension {dimension} is unsupported; using 0.5"),
            );
            return;
        }
        let closed_form = !text_parameter(&record.parameters, "aamode")
            .is_some_and(|mode| mode.eq_ignore_ascii_case("none"));
        self.compile_two_inputs(
            record,
            name,
            expected,
            depth,
            OP_CHECKERBOARD,
            closed_form as u32,
            [("tex1", 1.), ("tex2", 0.)],
        );
    }

    fn compile_checkerboard_3d(
        &mut self,
        record: &TextureRecord,
        name: &str,
        expected: ValueType,
        depth: usize,
    ) {
        let rows = match texture_from_world(record) {
            Ok(rows) => rows,
            Err(message) => {
                self.fallback(expected, format!("checkerboard {name} {message}"));
                return;
            }
        };
        for (input, default) in ["tex1", "tex2"].into_iter().zip([1., 0.]) {
            self.compile_operand(
                parameter(&record.parameters, input),
                expected,
                Some(literal_default(expected, default)),
                name,
                input,
                depth + 1,
            );
        }
        self.push_map(MAP_POINT, rows);
        self.push(TextureInstruction {
            op: OP_CHECKERBOARD3D,
            a: 0,
            b: 0,
            c: 0,
            value: [0.; 4],
        });
    }

    fn compile_bilerp(
        &mut self,
        record: &TextureRecord,
        name: &str,
        expected: ValueType,
        depth: usize,
    ) {
        let mapping = match self.mapping_2d(record) {
            Ok(mapping) => mapping,
            Err(message) => {
                self.fallback(expected, format!("bilerp {name} {message}"));
                return;
            }
        };
        for (corner, default) in [("v00", 0.), ("v01", 1.), ("v10", 0.), ("v11", 1.)] {
            self.compile_operand(
                parameter(&record.parameters, corner),
                expected,
                Some(literal_default(expected, default)),
                name,
                corner,
                depth + 1,
            );
        }
        let value = self.push_mapping(mapping);
        self.push(TextureInstruction {
            op: OP_BILERP,
            a: 0,
            b: 0,
            c: 0,
            value,
        });
    }

    fn compile_directionmix(
        &mut self,
        record: &TextureRecord,
        name: &str,
        expected: ValueType,
        depth: usize,
    ) {
        let dir = match parameter(&record.parameters, "dir").map(|p| p.floats.as_slice()) {
            Some(&[x, y, z]) => [x, y, z],
            _ => [0., 1., 0.],
        };
        // Matching r4, which keeps the transformed `dir` in render space.
        let m = &record.world_from_texture;
        let world: [f64; 3] = std::array::from_fn(|r| {
            m[r * 4] * dir[0] + m[r * 4 + 1] * dir[1] + m[r * 4 + 2] * dir[2]
        });
        let length = world.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !(length > 0. && length.is_finite()) {
            self.fallback(
                expected,
                format!("directionmix {name} has a degenerate dir"),
            );
            return;
        }
        for (input, default) in [("tex1", 0.), ("tex2", 1.)] {
            self.compile_operand(
                parameter(&record.parameters, input),
                expected,
                Some(literal_default(expected, default)),
                name,
                input,
                depth + 1,
            );
        }
        self.push(TextureInstruction {
            op: OP_DIRECTIONMIX,
            a: 0,
            b: 0,
            c: 0,
            value: [
                (world[0] / length) as f32,
                (world[1] / length) as f32,
                (world[2] / length) as f32,
                0.,
            ],
        });
    }

    fn compile_operand(
        &mut self,
        input: Option<&PreviewParameter>,
        expected: ValueType,
        default: Option<[f32; 4]>,
        texture_name: &str,
        input_name: &str,
        depth: usize,
    ) {
        let Some(input) = input else {
            if let Some(default) = default {
                self.push_constant(expected, default);
            } else {
                self.fallback(
                    expected,
                    format!("texture {texture_name} has no {input_name} input"),
                );
            }
            return;
        };

        if input.value_type.eq_ignore_ascii_case("texture") {
            if let Some(reference) = input.texts.first().filter(|name| !name.is_empty()) {
                self.compile_ref(reference, expected, depth);
            } else {
                self.invalid_fallback(
                    expected,
                    format!("texture {texture_name} has an empty {input_name} reference"),
                );
            }
            return;
        }

        let literal = match expected {
            ValueType::Float if input.value_type.eq_ignore_ascii_case("float") => {
                input.floats.first().map(|value| {
                    let value = *value as f32;
                    [value; 4]
                })
            }
            ValueType::Spectrum if input.value_type.eq_ignore_ascii_case("float") => input
                .floats
                .first()
                .map(|value| [*value as f32, *value as f32, *value as f32, 1.0]),
            ValueType::Spectrum
                if ["rgb", "color"]
                    .iter()
                    .any(|kind| input.value_type.eq_ignore_ascii_case(kind))
                    && matches!(input.floats.len(), 1 | 3) =>
            {
                input.floats.first().map(|first| {
                    let g = input.floats.get(1).copied().unwrap_or(*first) as f32;
                    let b = input.floats.get(2).copied().unwrap_or(*first) as f32;
                    [*first as f32, g, b, 1.0]
                })
            }
            _ => None,
        };
        if let Some(literal) = literal {
            self.push_constant(expected, literal);
        } else {
            self.invalid_graph = true;
            let kind = match expected {
                ValueType::Float => "float",
                ValueType::Spectrum => "float or rgb",
            };
            self.fallback(
                expected,
                format!(
                    "texture {texture_name} {input_name} must be a {kind} literal or texture reference"
                ),
            );
        }
    }

    fn push(&mut self, instruction: TextureInstruction) {
        if self.instructions.len() >= MAX_INSTRUCTIONS_PER_ROOT {
            self.budget_exceeded = true;
        } else {
            self.instructions.push(instruction);
        }
    }

    fn fallback(&mut self, value_type: ValueType, message: String) {
        self.diagnostics.push(message);
        self.push_constant(value_type, [0.5, 0.5, 0.5, 0.5]);
    }

    fn invalid_fallback(&mut self, value_type: ValueType, message: String) {
        self.invalid_graph = true;
        self.fallback(value_type, message);
    }
}

fn valid_stack(instructions: &[TextureInstruction]) -> bool {
    let mut depth = 0usize;
    let mut maximum = 0usize;
    for instruction in instructions {
        match instruction.op {
            OP_MAP => {}
            OP_FLOAT | OP_SPECTRUM | OP_IMAGE | OP_UV | OP_FBM | OP_WRINKLED | OP_WINDY
            | OP_MARBLE => depth += 1,
            OP_SCALE | OP_CHECKERBOARD | OP_CHECKERBOARD3D | OP_DOTS | OP_DIRECTIONMIX
                if depth >= 2 =>
            {
                depth -= 1
            }
            OP_MIX if depth >= 3 => depth -= 2,
            OP_BILERP if depth >= 4 => depth -= 3,
            _ => return false,
        }
        maximum = maximum.max(depth);
        if maximum > 32 {
            return false;
        }
    }
    depth == 1
}

/// A 2D texture's coordinates: `uscale vscale udelta vdelta` of its UVs, or a
/// `MAP_*` kind with its rows.
enum Mapping2D {
    Uv([f32; 4]),
    Mapped(u32, MappingRows),
}

const UV_IDENTITY: [f32; 4] = [1., 1., 0., 0.];

fn float_of(parameters: &[PreviewParameter], name: &str, default: f64) -> f64 {
    parameter(parameters, name)
        .and_then(|p| p.floats.first())
        .copied()
        .unwrap_or(default)
}

/// r4 maps through the inverse of the CTM at the `Texture` statement.
fn texture_from_world(record: &TextureRecord) -> Result<MappingRows, String> {
    let inverse = invert(&record.world_from_texture)
        .ok_or_else(|| "has a singular transform; using 0.5".to_string())?;
    Ok(std::array::from_fn(|r| {
        std::array::from_fn(|c| inverse[r * 4 + c])
    }))
}

/// The inverse of a row-major matrix, or None when it is singular.
pub(crate) fn invert(m: &[f64; 16]) -> Option<[f64; 16]> {
    let mut rows = [[0.; 8]; 4];
    for row in 0..4 {
        rows[row][..4].copy_from_slice(&m[row * 4..row * 4 + 4]);
        rows[row][row + 4] = 1.;
    }
    for column in 0..4 {
        let pivot =
            (column..4).max_by(|&a, &b| rows[a][column].abs().total_cmp(&rows[b][column].abs()))?;
        rows.swap(column, pivot);
        let divisor = rows[column][column];
        if divisor.abs() < 1e-12 || !divisor.is_finite() {
            return None;
        }
        for value in &mut rows[column] {
            *value /= divisor;
        }
        let pivot_row = rows[column];
        for (row, values) in rows.iter_mut().enumerate() {
            if row != column {
                let factor = values[column];
                for (value, pivot_value) in values.iter_mut().zip(pivot_row) {
                    *value -= factor * pivot_value;
                }
            }
        }
    }
    Some(std::array::from_fn(|i| rows[i / 4][i % 4 + 4]))
}

fn literal_default(expected: ValueType, value: f32) -> [f32; 4] {
    match expected {
        ValueType::Float => [value; 4],
        ValueType::Spectrum => [value, value, value, 1.],
    }
}

fn parameter<'a>(parameters: &'a [PreviewParameter], name: &str) -> Option<&'a PreviewParameter> {
    parameters.iter().find(|parameter| parameter.matches(name))
}

fn text_parameter<'a>(parameters: &'a [PreviewParameter], name: &str) -> Option<&'a str> {
    parameter(parameters, name).and_then(|parameter| parameter.texts.first().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        name: &str,
        kind: &str,
        implementation: &str,
        parameters: Vec<PreviewParameter>,
    ) -> TextureRecord {
        TextureRecord {
            name: name.into(),
            value_type: kind.into(),
            implementation: implementation.into(),
            world_from_texture: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            source: None,
            parameters,
        }
    }

    fn param(name: &str, value_type: &str, floats: &[f64], texts: &[&str]) -> PreviewParameter {
        PreviewParameter {
            name: name.into(),
            value_type: value_type.into(),
            floats: floats.to_vec(),
            texts: texts.iter().map(|text| (*text).into()).collect(),
            truncated: false,
        }
    }

    #[test]
    fn compiles_typed_constant_and_same_name_namespaces() {
        let records = vec![
            record(
                "surface",
                "float",
                "constant",
                vec![param("value", "float", &[0.25], &[])],
            ),
            record(
                "surface",
                "spectrum",
                "constant",
                vec![param("value", "rgb", &[0.1, 0.2, 0.3], &[])],
            ),
        ];
        let float = compile("surface", ValueType::Float, &records);
        let spectrum = compile("surface", ValueType::Spectrum, &records);
        assert_eq!(float.instructions[0].op, OP_FLOAT);
        assert_eq!(float.instructions[0].value, [0.25; 4]);
        assert_eq!(spectrum.instructions[0].value, [0.1, 0.2, 0.3, 1.0]);
        assert!(float.diagnostics.is_empty());
    }

    #[test]
    fn constants_use_pbrt_defaults_and_diagnose_unsupported_spectrum_forms() {
        let float_default = compile(
            "float_default",
            ValueType::Float,
            &[record("float_default", "float", "constant", vec![])],
        );
        assert_eq!(float_default.instructions[0].value, [1.0; 4]);

        let spectrum_default = compile(
            "spectrum_default",
            ValueType::Spectrum,
            &[record("spectrum_default", "spectrum", "constant", vec![])],
        );
        assert_eq!(spectrum_default.instructions[0].value, [1.0; 4]);

        for value in [
            param("value", "spectrum", &[], &["named-sample"]),
            param("value", "rgb", &[0.1, 0.2], &[]),
            param("value", "rgb", &[0.1, 0.2, 0.3, 0.4], &[]),
        ] {
            let unsupported = record("unsupported", "spectrum", "constant", vec![value]);
            let program = compile("unsupported", ValueType::Spectrum, &[unsupported]);
            assert_eq!(program.instructions.len(), 1);
            assert_eq!(program.instructions[0].op, OP_SPECTRUM);
            assert_eq!(program.instructions[0].value, [0.5; 4]);
            assert!(
                program
                    .diagnostics
                    .iter()
                    .any(|note| note.contains("constant texture unsupported value"))
            );
        }
    }

    #[test]
    fn compiles_scale_mix_and_image_dependencies_postfix() {
        let records = vec![
            record(
                "red",
                "spectrum",
                "constant",
                vec![param("value", "rgb", &[1., 0., 0.], &[])],
            ),
            record(
                "image",
                "spectrum",
                "imagemap",
                vec![param("filename", "string", &[], &["tex.png"])],
            ),
            record(
                "scaled",
                "spectrum",
                "scale",
                vec![
                    param("tex", "texture", &[], &["image"]),
                    param("scale", "float", &[0.25], &[]),
                ],
            ),
        ];
        let program = compile("scaled", ValueType::Spectrum, &records);
        assert_eq!(program.images, ["texture/spectrum/image"]);
        assert_eq!(
            program
                .instructions
                .iter()
                .map(|instruction| instruction.op)
                .collect::<Vec<_>>(),
            [OP_IMAGE, OP_FLOAT, OP_SCALE]
        );
        assert!(program.diagnostics.is_empty());
    }

    #[test]
    fn mix_accepts_rgb_literals_defaults_and_unclamped_float_amounts() {
        let literal_mix = record(
            "literal_mix",
            "spectrum",
            "mix",
            vec![
                param("tex1", "rgb", &[0.25, 0.5, 0.75], &[]),
                param("amount", "float", &[1.5], &[]),
            ],
        );
        let program = compile("literal_mix", ValueType::Spectrum, &[literal_mix]);
        assert_eq!(
            program
                .instructions
                .iter()
                .map(|instruction| instruction.op)
                .collect::<Vec<_>>(),
            [OP_SPECTRUM, OP_SPECTRUM, OP_FLOAT, OP_MIX]
        );
        assert_eq!(program.instructions[0].value, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(program.instructions[1].value, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(program.instructions[2].value, [1.5; 4]);
        assert!(program.diagnostics.is_empty());

        let defaults = compile(
            "defaults",
            ValueType::Spectrum,
            &[record("defaults", "spectrum", "mix", vec![])],
        );
        assert_eq!(defaults.instructions[0].value, [1.0; 4]);
        assert_eq!(defaults.instructions[1].value, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(defaults.instructions[2].value, [0.5; 4]);
        assert!(defaults.diagnostics.is_empty());

        let float_defaults = compile(
            "float_defaults",
            ValueType::Float,
            &[record("float_defaults", "float", "mix", vec![])],
        );
        assert_eq!(float_defaults.instructions[0].value, [0.0; 4]);
        assert_eq!(float_defaults.instructions[1].value, [1.0; 4]);
        assert_eq!(float_defaults.instructions[2].value, [0.5; 4]);
    }

    #[test]
    fn mix_accepts_a_float_texture_amount() {
        let records = vec![
            record(
                "blue",
                "spectrum",
                "constant",
                vec![param("value", "rgb", &[0.0, 0.0, 1.0], &[])],
            ),
            record(
                "amount",
                "float",
                "constant",
                vec![param("value", "float", &[0.75], &[])],
            ),
            record(
                "mix",
                "spectrum",
                "mix",
                vec![
                    param("tex1", "rgb", &[1.0, 0.0, 0.0], &[]),
                    param("tex2", "texture", &[], &["blue"]),
                    param("amount", "texture", &[], &["amount"]),
                ],
            ),
        ];
        let program = compile("mix", ValueType::Spectrum, &records);
        assert_eq!(
            program
                .instructions
                .iter()
                .map(|instruction| instruction.op)
                .collect::<Vec<_>>(),
            [OP_SPECTRUM, OP_SPECTRUM, OP_FLOAT, OP_MIX]
        );
        assert_eq!(program.instructions[2].value, [0.75; 4]);
        assert!(program.diagnostics.is_empty());
    }

    #[test]
    fn scale_uses_v4_tex_scale_and_reports_legacy_aliases() {
        let red = record(
            "red",
            "spectrum",
            "constant",
            vec![param("value", "rgb", &[0.8, 0.2, 0.1], &[])],
        );
        let modern = record(
            "modern",
            "spectrum",
            "scale",
            vec![
                param("tex", "texture", &[], &["red"]),
                param("scale", "float", &[0.25], &[]),
            ],
        );
        let program = compile("modern", ValueType::Spectrum, &[red.clone(), modern]);
        assert_eq!(
            program
                .instructions
                .iter()
                .map(|instruction| instruction.op)
                .collect::<Vec<_>>(),
            [OP_SPECTRUM, OP_FLOAT, OP_SCALE]
        );
        assert_eq!(program.instructions[1].value, [0.25; 4]);
        assert!(program.diagnostics.is_empty());

        let literal_tex = record(
            "literal_tex",
            "spectrum",
            "scale",
            vec![
                param("tex", "rgb", &[0.4, 0.5, 0.6], &[]),
                param("scale", "float", &[0.5], &[]),
            ],
        );
        let program = compile("literal_tex", ValueType::Spectrum, &[literal_tex]);
        assert_eq!(program.instructions[0].value, [0.4, 0.5, 0.6, 1.0]);
        assert_eq!(program.instructions[1].value, [0.5; 4]);

        let all_defaults = record("all_defaults", "float", "scale", vec![]);
        let program = compile("all_defaults", ValueType::Float, &[all_defaults]);
        assert_eq!(program.instructions.len(), 3);
        assert_eq!(program.instructions[0].value, [1.0; 4]);
        assert_eq!(program.instructions[1].value, [1.0; 4]);

        let legacy = record(
            "legacy",
            "spectrum",
            "scale",
            vec![
                param("tex1", "texture", &[], &["red"]),
                param("tex2", "rgb", &[0.5, 0.25, 1.0], &[]),
            ],
        );
        let program = compile("legacy", ValueType::Spectrum, &[red, legacy]);
        assert_eq!(program.instructions[1].op, OP_SPECTRUM);
        assert_eq!(program.instructions[1].value, [0.5, 0.25, 1.0, 1.0]);
        assert!(
            program
                .diagnostics
                .iter()
                .any(|note| note.contains("legacy tex1/tex2"))
        );

        let default_scale = record(
            "default_scale",
            "float",
            "scale",
            vec![param("tex", "float", &[0.4], &[])],
        );
        let program = compile("default_scale", ValueType::Float, &[default_scale]);
        assert_eq!(program.instructions[0].value, [0.4; 4]);
        assert_eq!(program.instructions[1].value, [1.0; 4]);
    }

    #[test]
    fn non_uv_imagemap_mappings_fallback_with_a_diagnostic() {
        let image = record(
            "toroidal",
            "spectrum",
            "imagemap",
            vec![
                param("filename", "string", &[], &["sphere.png"]),
                param("mapping", "string", &[], &["toroidal"]),
            ],
        );
        let program = compile("toroidal", ValueType::Spectrum, &[image]);
        assert!(program.images.is_empty());
        assert_eq!(program.instructions.len(), 1);
        assert_eq!(program.instructions[0].op, OP_SPECTRUM);
        assert!(
            program
                .diagnostics
                .iter()
                .any(|note| note.contains("mapping toroidal is unsupported"))
        );
    }

    #[test]
    fn image_instruction_carries_wrap_uv_mapping_and_intensity() {
        let records = vec![record(
            "mapped",
            "spectrum",
            "imagemap",
            vec![
                param("filename", "string", &[], &["mapped.png"]),
                param("wrap", "string", &[], &["black"]),
                param("scale", "float", &[0.25], &[]),
                param("uscale", "float", &[2.0], &[]),
                param("vscale", "float", &[3.0], &[]),
                param("udelta", "float", &[-0.5], &[]),
                param("vdelta", "float", &[0.125], &[]),
            ],
        )];
        let program = compile("mapped", ValueType::Spectrum, &records);
        let instruction = program.instructions[0];
        assert_eq!(instruction.op, OP_IMAGE);
        assert_eq!(instruction.b, 2);
        assert_eq!(f32::from_bits(instruction.c), 0.25);
        assert_eq!(instruction.value, [2.0, 3.0, -0.5, 0.125]);
    }

    #[test]
    fn float_imagemaps_flag_the_scalar_widening_bit() {
        let float = vec![record(
            "mask",
            "float",
            "imagemap",
            vec![
                param("filename", "string", &[], &["mask.png"]),
                param("wrap", "string", &[], &["black"]),
            ],
        )];
        let program = compile("mask", ValueType::Float, &float);
        // The wrap stays in the low byte; bit 8 tells the shader this
        // sample is one scalar the R channel widens to RGB.
        assert_eq!(program.instructions[0].b, 0x102);
        let spectrum = vec![record(
            "mask",
            "spectrum",
            "imagemap",
            vec![
                param("filename", "string", &[], &["mask.png"]),
                param("wrap", "string", &[], &["black"]),
            ],
        )];
        let program = compile("mask", ValueType::Spectrum, &spectrum);
        assert_eq!(program.instructions[0].b, 0x2);
    }

    #[test]
    fn cycles_and_missing_references_produce_diagnostic_constant_fallbacks() {
        let records = vec![record(
            "loop",
            "float",
            "scale",
            vec![
                param("tex1", "texture", &[], &["loop"]),
                param("tex2", "texture", &[], &["missing"]),
            ],
        )];
        let program = compile("loop", ValueType::Float, &records);
        assert!(
            program
                .diagnostics
                .iter()
                .any(|message| message.contains("cycle"))
        );
        assert!(
            program
                .diagnostics
                .iter()
                .any(|message| message.contains("missing"))
        );
        assert_eq!(program.instructions.len(), 1);
        assert_eq!(program.instructions[0].op, OP_FLOAT);
        assert_eq!(program.instructions[0].value, [0.5; 4]);
    }

    #[test]
    fn excessive_reference_depth_replaces_the_whole_root() {
        let mut records = (0..=MAX_REFERENCE_DEPTH)
            .map(|index| {
                if index == MAX_REFERENCE_DEPTH {
                    record(
                        &format!("n{index}"),
                        "float",
                        "constant",
                        vec![param("value", "float", &[0.9], &[])],
                    )
                } else {
                    record(
                        &format!("n{index}"),
                        "float",
                        "scale",
                        vec![
                            param("tex", "texture", &[], &[&format!("n{}", index + 1)]),
                            param("scale", "float", &[2.0], &[]),
                        ],
                    )
                }
            })
            .collect::<Vec<_>>();
        records.reverse();
        let program = compile("n0", ValueType::Float, &records);
        assert_eq!(program.instructions.len(), 1);
        assert_eq!(program.instructions[0].op, OP_FLOAT);
        assert_eq!(program.instructions[0].value, [0.5; 4]);
        assert!(
            program
                .diagnostics
                .iter()
                .any(|note| note.contains("reference depth exceeds"))
        );
    }

    #[test]
    fn stack_validator_accepts_32_values_and_rejects_33() {
        let value = TextureInstruction {
            op: OP_FLOAT,
            a: 0,
            b: 0,
            c: 0,
            value: [1.0; 4],
        };
        let scale = TextureInstruction {
            op: OP_SCALE,
            ..value
        };
        let valid = [vec![value; 32], vec![scale; 31]].concat();
        assert!(valid_stack(&valid));
        let invalid = [vec![value; 33], vec![scale; 32]].concat();
        assert!(!valid_stack(&invalid));
    }

    #[test]
    fn instruction_layout_matches_wgsl_storage_stride() {
        assert_eq!(std::mem::size_of::<TextureInstruction>(), 32);
        assert_eq!(std::mem::align_of::<TextureInstruction>(), 16);
    }

    #[test]
    fn oversized_dag_stops_expansion_and_returns_a_valid_constant() {
        let mut records = vec![record(
            "leaf",
            "float",
            "constant",
            vec![param("value", "float", &[0.75], &[])],
        )];
        // A shared DAG used to expand exponentially because every reference
        // was recursively inlined. The instruction budget must stop that
        // expansion before allocating an unbounded temporary program.
        for index in 0..20 {
            let child = if index == 0 {
                "leaf".to_owned()
            } else {
                format!("node{}", index - 1)
            };
            records.push(record(
                &format!("node{index}"),
                "float",
                "scale",
                vec![
                    param("tex1", "texture", &[], &[&child]),
                    param("tex2", "texture", &[], &[&child]),
                ],
            ));
        }
        records.push(record(
            "root",
            "float",
            "scale",
            vec![
                param("tex1", "texture", &[], &["node19"]),
                param("tex2", "texture", &[], &["node19"]),
            ],
        ));
        let program = compile("root", ValueType::Float, &records);
        assert_eq!(program.instructions.len(), 1);
        assert_eq!(program.instructions[0].op, OP_FLOAT);
        assert!(
            program
                .diagnostics
                .iter()
                .any(|message| message.contains("exceeds 256 instructions"))
        );
    }

    fn ops(program: &TextureProgram) -> Vec<u32> {
        program.instructions.iter().map(|i| i.op).collect()
    }

    /// Earlier r4 read tex1 and tex2 here; scenes using them now change color,
    /// which must not happen silently.
    #[test]
    fn dots_tex1_and_tex2_are_reported() {
        let parameter = PreviewParameter {
            name: "tex1".into(),
            value_type: "rgb".into(),
            floats: vec![1., 0., 0.],
            texts: vec![],
            truncated: false,
        };
        let noted = compile(
            "d",
            ValueType::Spectrum,
            &[record("d", "spectrum", "dots", vec![parameter])],
        );
        assert!(
            noted.diagnostics.iter().any(|d| d.contains("tex1")),
            "{:?}",
            noted.diagnostics
        );
    }

    #[test]
    fn dots_read_inside_then_outside_like_r4() {
        let program = compile(
            "d",
            ValueType::Float,
            &[record(
                "d",
                "float",
                "dots",
                vec![
                    param("outside", "float", &[0.25], &[]),
                    param("inside", "float", &[0.75], &[]),
                ],
            )],
        );
        assert_eq!(program.instructions[0].value[0], 0.75);
        assert_eq!(program.instructions[1].value[0], 0.25);
    }

    /// r4 defaults: checkerboard tex1 = 1, tex2 = 0; dots inside = 1, outside = 0; bilerp corners
    /// 0, 1, 0, 1; directionmix tex1 = 0, tex2 = 1, dir = +y.
    #[test]
    fn procedural_two_d_textures_compile_with_r4_defaults() {
        let checker = record(
            "c",
            "spectrum",
            "checkerboard",
            vec![
                param("uscale", "float", &[4.], &[]),
                param("vdelta", "float", &[0.5], &[]),
            ],
        );
        let program = compile("c", ValueType::Spectrum, &[checker]);
        assert_eq!(ops(&program), [OP_SPECTRUM, OP_SPECTRUM, OP_CHECKERBOARD]);
        assert_eq!(program.instructions[0].value, [1., 1., 1., 1.]);
        assert_eq!(program.instructions[1].value, [0., 0., 0., 1.]);
        assert_eq!(
            program.instructions[2].a, 1,
            "aamode defaults to closedform"
        );
        assert_eq!(program.instructions[2].value, [4., 1., 0., 0.5]);

        let none = record(
            "c",
            "float",
            "checkerboard",
            vec![param("aamode", "string", &[], &["none"])],
        );
        assert_eq!(compile("c", ValueType::Float, &[none]).instructions[2].a, 0);

        let dots = compile(
            "d",
            ValueType::Float,
            &[record("d", "float", "dots", vec![])],
        );
        assert_eq!(ops(&dots), [OP_FLOAT, OP_FLOAT, OP_DOTS]);
        assert_eq!(dots.instructions[0].value[0], 1.);
        assert_eq!(dots.instructions[1].value[0], 0.);

        let bilerp = compile(
            "b",
            ValueType::Float,
            &[record("b", "float", "bilerp", vec![])],
        );
        assert_eq!(
            ops(&bilerp),
            [OP_FLOAT, OP_FLOAT, OP_FLOAT, OP_FLOAT, OP_BILERP]
        );
        assert_eq!(
            bilerp.instructions[..4]
                .iter()
                .map(|i| i.value[0])
                .collect::<Vec<_>>(),
            [0., 1., 0., 1.]
        );

        let uv = compile(
            "u",
            ValueType::Spectrum,
            &[record("u", "spectrum", "uv", vec![])],
        );
        assert_eq!(ops(&uv), [OP_UV]);
    }

    /// `dir` is transformed by the texture's CTM and normalized, as r4 does.
    #[test]
    fn directionmix_transforms_dir_by_the_declaration_ctm() {
        let mut texture = record(
            "m",
            "float",
            "directionmix",
            vec![param("dir", "vector3", &[0., 3., 0.], &[])],
        );
        // Rotate +y onto +z.
        texture.world_from_texture = [
            1., 0., 0., 9., 0., 0., -1., 9., 0., 1., 0., 9., 0., 0., 0., 1.,
        ];
        let program = compile("m", ValueType::Float, &[texture]);
        assert_eq!(ops(&program), [OP_FLOAT, OP_FLOAT, OP_DIRECTIONMIX]);
        assert_eq!(program.instructions[0].value[0], 0.);
        assert_eq!(program.instructions[1].value[0], 1.);
        assert_eq!(&program.instructions[2].value[..3], [0., 0., 1.]);
    }

    /// Unsupported mappings and dimensions fall back with a diagnostic
    /// rather than rendering as UV.
    #[test]
    fn unknown_mappings_and_dimensions_fall_back_with_a_note() {
        for texture in [
            record(
                "t",
                "float",
                "checkerboard",
                vec![param("dimension", "integer", &[4.], &[])],
            ),
            record(
                "t",
                "float",
                "dots",
                vec![param("mapping", "string", &[], &["toroidal"])],
            ),
        ] {
            let program = compile("t", ValueType::Float, &[texture]);
            assert_eq!(program.instructions[0].value[0], 0.5);
            assert_eq!(program.instructions.len(), 1);
            assert!(!program.diagnostics.is_empty());
        }
        let float_uv = compile("u", ValueType::Float, &[record("u", "float", "uv", vec![])]);
        assert_eq!(ops(&float_uv), [OP_FLOAT], "uv is a spectrum texture only");
        assert!(!float_uv.diagnostics.is_empty());
    }

    fn scaled_record(implementation: &str, parameters: Vec<PreviewParameter>) -> TextureRecord {
        let mut texture = record("t", "spectrum", implementation, parameters);
        // Translate (10, 0, 0), then scale by 2: texture = (world - (10, 0, 0)) / 2.
        texture.world_from_texture = [
            2., 0., 0., 10., 0., 2., 0., 0., 0., 0., 2., 0., 0., 0., 0., 1.,
        ];
        texture
    }

    #[test]
    fn mapped_textures_invert_the_declaration_transform() {
        let program = compile(
            "t",
            ValueType::Spectrum,
            &[scaled_record(
                "bilerp",
                vec![param("mapping", "string", &[], &["spherical"])],
            )],
        );
        assert_eq!(program.diagnostics, Vec::<String>::new());
        let map = program.instructions[program.instructions.len() - 2];
        assert_eq!((map.op, map.a), (OP_MAP, MAP_SPHERICAL));
        assert_eq!(program.mappings[map.b as usize][0], [0.5, 0., 0., -5.]);
    }

    /// r4's planar: s = udelta + texture_point · v1.
    #[test]
    fn planar_folds_its_axes_and_deltas_into_the_rows() {
        let program = compile(
            "t",
            ValueType::Spectrum,
            &[scaled_record(
                "bilerp",
                vec![
                    param("mapping", "string", &[], &["planar"]),
                    param("v1", "vector3", &[0., 1., 0.], &[]),
                    param("vdelta", "float", &[3.], &[]),
                    param("udelta", "float", &[1.], &[]),
                ],
            )],
        );
        assert_eq!(program.mappings[0][0], [0., 0.5, 0., 1.]);
        assert_eq!(program.mappings[0][1], [0., 0.5, 0., 3.]);
    }

    /// A mapped parent's own map instruction comes right before it, after its
    /// inputs' maps, so an input cannot steal or overwrite it.
    #[test]
    fn nested_mappings_stay_with_their_own_textures() {
        let child = record("child", "spectrum", "fbm", vec![]);
        let parent = record(
            "t",
            "spectrum",
            "checkerboard",
            vec![
                param("mapping", "string", &[], &["cylindrical"]),
                param("tex1", "texture", &[], &["child"]),
            ],
        );
        let program = compile("t", ValueType::Spectrum, &[parent, child]);
        assert_eq!(
            ops(&program),
            [OP_MAP, OP_FBM, OP_SPECTRUM, OP_MAP, OP_CHECKERBOARD]
        );
        assert_eq!(program.instructions[0].a, MAP_POINT);
        assert_eq!(program.instructions[3].a, MAP_CYLINDRICAL);
        assert_eq!(program.mappings.len(), 1, "same transform, one row table");
    }

    #[test]
    fn solid_textures_compile_with_r4_defaults_and_bounds() {
        let fbm = compile(
            "t",
            ValueType::Float,
            &[record("t", "float", "fbm", vec![])],
        );
        assert_eq!(ops(&fbm), [OP_MAP, OP_FBM]);
        assert_eq!(
            (fbm.instructions[1].a, fbm.instructions[1].value[0]),
            (8, 0.5)
        );
        let huge = compile(
            "t",
            ValueType::Float,
            &[record(
                "t",
                "float",
                "wrinkled",
                vec![param("octaves", "integer", &[1e6], &[])],
            )],
        );
        assert_eq!(huge.instructions[1].a, 32);
        let marble = compile(
            "t",
            ValueType::Spectrum,
            &[record("t", "spectrum", "marble", vec![])],
        );
        assert_eq!(marble.instructions[1].value, [0.5, 1., 0.2, 0.]);
        let float_marble = compile(
            "t",
            ValueType::Float,
            &[record("t", "float", "marble", vec![])],
        );
        assert_eq!(ops(&float_marble), [OP_FLOAT], "r4 has no float marble");
        let checker = compile(
            "t",
            ValueType::Float,
            &[record(
                "t",
                "float",
                "checkerboard",
                vec![param("dimension", "integer", &[3.], &[])],
            )],
        );
        assert_eq!(
            ops(&checker),
            [OP_FLOAT, OP_FLOAT, OP_MAP, OP_CHECKERBOARD3D]
        );
    }

    #[test]
    fn singular_transforms_fall_back_with_a_note() {
        let mut flat = record("t", "float", "fbm", vec![]);
        flat.world_from_texture[10] = 0.;
        let program = compile("t", ValueType::Float, &[flat]);
        assert_eq!(ops(&program), [OP_FLOAT]);
        assert!(program.diagnostics[0].contains("singular"));
    }

    /// Mirrors r4's NOISEPERM: 256 entries that are a permutation of 0..256.
    #[test]
    fn noise_table_is_r4_permutation() {
        assert_eq!(&NOISE_PERM[..4], [151, 160, 137, 91]);
        assert_eq!(NOISE_PERM[255], 180);
        let mut sorted = NOISE_PERM;
        sorted.sort_unstable();
        assert!(sorted.iter().enumerate().all(|(i, v)| i as u32 == *v));
    }
}
