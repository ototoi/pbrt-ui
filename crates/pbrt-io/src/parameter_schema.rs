//! The per-implementation parameter table follows pbrt-v4 `create()` functions.
//! Where the repository's parser and pbrt-v4 differ, pbrt-v4 behavior takes precedence.
//!
//! The table is **information, never a restriction**: implementations outside
//! it stay legal with today's free-form JSON parameter editing. It feeds the
//! editors (FX8a's type-aware parameter rows, FX5's create dialogs, FX7's
//! keep-with-warning migration) and `project.get_parameter_schema`.
use project::Parameter;
use project::TextureValueType;

/// What a parameter is used for, the way editors treat it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Plain values.
    Value,
    /// Accepts a texture reference; `texture` says which lineage.
    TextureInput,
    /// A path managed by the dependency schema (the doc's file role).
    File,
    /// References other project items.
    Resource,
}

/// Which texture lineage a `role: TextureInput` parameter accepts — the
/// doc's FT/ST, with r4's spectrum interpretation classes in parentheses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureInput {
    Float,
    SpectrumAlbedo,
    SpectrumUnbounded,
    SpectrumIlluminant,
}

/// How a default that follows another parameter is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependsOperation {
    /// The same value (roughness → uroughness, radius → zmax).
    Same,
    /// Negated (radius → zmin).
    Negated,
    /// 0..n−1 over another array's length (bilinearmesh indices from P).
    SequentialIndices,
    /// Chosen by a file's extension (imagemap encoding from filename).
    FileEncoding,
}

/// The default of one parameter: a constant, or something r4 resolves.
#[derive(Clone, Debug, PartialEq)]
pub enum DefaultValue {
    /// Constant floats (one value for scalars, three for a color / point).
    Floats(&'static [f64]),
    Integers(&'static [i64]),
    Booleans(&'static [bool]),
    /// Plain strings, or builtin spectrum names for spectrum parameters.
    Strings(&'static [&'static str]),
    /// The color space's illuminant (r4 lights' I / L).
    Illuminant,
    /// Follows another parameter (which one + how).
    Depends {
        parameter: &'static str,
        operation: DependsOperation,
    },
    /// The medium preset's sigma, else these floats (r4 prefers explicit
    /// sigma over the preset; pbrt-v4 the other way round).
    PresetElse(&'static [f64]),
}

/// One parameter of one implementation.
#[derive(Clone, Debug)]
pub struct ParameterSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// The declared PBRT types accepted for this parameter.
    pub value_types: &'static [&'static str],
    pub default: Option<DefaultValue>,
    /// Some when the parameter accepts textures; the kind is the lineage.
    pub texture: Option<TextureInput>,
    pub role: Role,
    pub required: bool,
    /// The Inspector component this row belongs to instead of the body.
    pub group: Option<Group>,
    /// r4 reads this row only while every condition holds; empty = always.
    pub when: &'static [Condition],
    /// The values a string row may take; empty = free text.
    pub choices: &'static [&'static str],
    /// Common values of a free-text row (offered, not enforced).
    pub suggestions: &'static [&'static str],
}

/// A component a texture's rows split into (the body is the rest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// The 2D texture mapping: `mapping` and the rows it selects.
    Mapping,
}

impl Group {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mapping => "mapping",
        }
    }
}

/// `parameter`'s value (declared, else its default) is one of `values`;
/// integers compare in decimal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Condition {
    pub parameter: &'static str,
    pub values: &'static [&'static str],
}

/// An all-empty base so const rows only spell what the doc's table says.
const EMPTY: ParameterSpec = ParameterSpec {
    name: "",
    aliases: &[],
    value_types: &[],
    default: None,
    texture: None,
    role: Role::Value,
    required: false,
    group: None,
    when: &[],
    choices: &[],
    suggestions: &[],
};

/// A plain value row.
const fn value(
    name: &'static str,
    value_types: &'static [&'static str],
    default: Option<DefaultValue>,
) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types,
        default,
        ..EMPTY
    }
}

/// A file-path row (`role: file`).
const fn file(
    name: &'static str,
    value_types: &'static [&'static str],
    default: Option<DefaultValue>,
) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types,
        default,
        role: Role::File,
        ..EMPTY
    }
}

/// A float value that also accepts a float texture (the doc's FT).
const fn float_ft(name: &'static str, default: Option<DefaultValue>) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types: &["float"],
        default,
        texture: Some(TextureInput::Float),
        role: Role::TextureInput,
        ..EMPTY
    }
}

/// A float row with the doc's standard float-texture default.
const fn float_ft_default(name: &'static str, values: &'static [f64]) -> ParameterSpec {
    float_ft(name, Some(DefaultValue::Floats(values)))
}

/// A spectrum value that accepts a spectrum texture (the doc's ST).
const fn spectrum_st(
    name: &'static str,
    default: Option<DefaultValue>,
    kind: TextureInput,
) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types: &["spectrum", "rgb"],
        default,
        texture: Some(kind),
        role: Role::TextureInput,
        ..EMPTY
    }
}

/// A plain constant `float` row.
const fn float_row(name: &'static str, values: &'static [f64]) -> ParameterSpec {
    value(name, &["float"], Some(DefaultValue::Floats(values)))
}

/// A plain constant `integer` row.
const fn integer_row(name: &'static str, values: &'static [i64]) -> ParameterSpec {
    value(name, &["integer"], Some(DefaultValue::Integers(values)))
}

/// A plain constant `bool` row.
const fn bool_row(name: &'static str, values: &'static [bool]) -> ParameterSpec {
    value(name, &["bool"], Some(DefaultValue::Booleans(values)))
}

/// A required row with no default.
const fn required(name: &'static str, value_types: &'static [&'static str]) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types,
        required: true,
        ..EMPTY
    }
}

/// A texture operand: accepts a value or a texture of `kind`.
const fn tex_operand(
    name: &'static str,
    aliases: &'static [&'static str],
    value_types: &'static [&'static str],
    default: Option<DefaultValue>,
    kind: TextureInput,
) -> ParameterSpec {
    ParameterSpec {
        name,
        aliases,
        value_types,
        default,
        texture: Some(kind),
        role: Role::TextureInput,
        ..EMPTY
    }
}

/// A float operand whose default follows another parameter (roughness →
/// uroughness); still accepts a float texture (the doc's FT column).
const fn depends_ft(name: &'static str, parameter: &'static str) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types: &["float"],
        default: Some(DefaultValue::Depends {
            parameter,
            operation: DependsOperation::Same,
        }),
        texture: Some(TextureInput::Float),
        role: Role::TextureInput,
        ..EMPTY
    }
}

/// The lights' I / L: r4's default is the color space illuminant.
const fn illuminant(name: &'static str) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types: &["spectrum", "rgb", "blackbody"],
        default: Some(DefaultValue::Illuminant),
        ..EMPTY
    }
}

/// A common-row alpha / shadowalpha pair used by every shape.
const fn shape_mask(name: &'static str) -> ParameterSpec {
    float_ft(name, None)
}

/// The two light direction defaults.
const FROM: &[f64] = &[0., 0., 0.];
const TO: &[f64] = &[0., 0., 1.];
const UP: &[f64] = &[0., 1., 0.];
const PLANAR_U: &[f64] = &[1., 0., 0.];
const PLANAR_V: &[f64] = &[0., 1., 0.];

// ---------------------------------------------------------------- shapes

const SPHERE: &[ParameterSpec] = &[
    float_row("radius", &[1.]),
    // The defaults of zmin / zmax follow radius.
    value(
        "zmin",
        &["float"],
        Some(DefaultValue::Depends {
            parameter: "radius",
            operation: DependsOperation::Negated,
        }),
    ),
    // Shape scalars take plain values; only alpha / shadowalpha take
    // textures, which are common to all shapes.
    value(
        "zmax",
        &["float"],
        Some(DefaultValue::Depends {
            parameter: "radius",
            operation: DependsOperation::Same,
        }),
    ),
    float_row("phimax", &[360.]),
    shape_mask("alpha"),
    shape_mask("shadowalpha"),
];

const DISK: &[ParameterSpec] = &[
    float_row("height", &[0.]),
    float_row("radius", &[1.]),
    float_row("innerradius", &[0.]),
    float_row("phimax", &[360.]),
    shape_mask("alpha"),
    shape_mask("shadowalpha"),
];

const CYLINDER: &[ParameterSpec] = &[
    float_row("radius", &[1.]),
    float_row("zmin", &[-1.]),
    float_row("zmax", &[1.]),
    float_row("phimax", &[360.]),
    shape_mask("alpha"),
    shape_mask("shadowalpha"),
];

// r4 (unlike pbrt-v4) does not let indices default away, and adds twosided.
const TRIANGLEMESH: &[ParameterSpec] = &[
    required("indices", &["integer"]),
    required("P", &["point3"]),
    ParameterSpec {
        name: "uv",
        aliases: &["st"],
        value_types: &["point2"],
        ..EMPTY
    },
    value("S", &["vector"], None),
    value("N", &["normal"], None),
    bool_row("twosided", &[true]),
    shape_mask("alpha"),
    shape_mask("shadowalpha"),
];

const BILINEARMESH: &[ParameterSpec] = &[
    required("P", &["point3"]),
    value(
        "indices",
        &["integer"],
        Some(DefaultValue::Depends {
            parameter: "P",
            operation: DependsOperation::SequentialIndices,
        }),
    ),
    value("N", &["normal"], None),
    ParameterSpec {
        name: "uv",
        aliases: &["st"],
        value_types: &["point2"],
        ..EMPTY
    },
    value("faceIndices", &["integer"], None),
    file(
        "emissionfilename",
        &["string"],
        Some(DefaultValue::Strings(&[""])),
    ),
    shape_mask("alpha"),
    shape_mask("shadowalpha"),
];

const PLYMESH: &[ParameterSpec] = &[
    ParameterSpec {
        name: "filename",
        value_types: &["string"],
        role: Role::File,
        required: true,
        ..EMPTY
    },
    float_ft("displacement", None),
    float_row("edgelength", &[1.]),
    bool_row("twosided", &[true]),
    shape_mask("alpha"),
    shape_mask("shadowalpha"),
];

// -------------------------------------------------------------- materials
// Every material but mix shares displacement and normalmap; the
// rows are spelled per table so each stays one self-contained slice.

const fn displacement_row() -> ParameterSpec {
    float_ft("displacement", None)
}

const fn normalmap_row() -> ParameterSpec {
    file("normalmap", &["string"], Some(DefaultValue::Strings(&[""])))
}

const DIFFUSE: &[ParameterSpec] = &[
    spectrum_st(
        "reflectance",
        Some(DefaultValue::Floats(&[0.5])),
        TextureInput::SpectrumAlbedo,
    ),
    displacement_row(),
    normalmap_row(),
];

const CONDUCTOR: &[ParameterSpec] = &[
    ParameterSpec {
        name: "eta",
        value_types: &["spectrum", "rgb", "float"],
        default: Some(DefaultValue::Strings(&["metal-Cu-eta"])),
        texture: Some(TextureInput::SpectrumUnbounded),
        role: Role::TextureInput,
        ..EMPTY
    },
    ParameterSpec {
        name: "k",
        value_types: &["spectrum", "rgb", "float"],
        default: Some(DefaultValue::Strings(&["metal-Cu-k"])),
        texture: Some(TextureInput::SpectrumUnbounded),
        role: Role::TextureInput,
        ..EMPTY
    },
    spectrum_st("reflectance", None, TextureInput::SpectrumAlbedo),
    float_ft_default("roughness", &[0.]),
    depends_ft("uroughness", "roughness"),
    depends_ft("vroughness", "roughness"),
    bool_row("remaproughness", &[true]),
    displacement_row(),
    normalmap_row(),
];

const DIELECTRIC: &[ParameterSpec] = &[
    // r4 errors when eta is given a texture; the table treats it as constant.
    value(
        "eta",
        &["float", "spectrum"],
        Some(DefaultValue::Floats(&[1.5])),
    ),
    float_ft_default("roughness", &[0.]),
    depends_ft("uroughness", "roughness"),
    depends_ft("vroughness", "roughness"),
    bool_row("remaproughness", &[true]),
    displacement_row(),
    normalmap_row(),
];

const COATEDDIFFUSE: &[ParameterSpec] = &[
    spectrum_st(
        "reflectance",
        Some(DefaultValue::Floats(&[0.5])),
        TextureInput::SpectrumAlbedo,
    ),
    value(
        "eta",
        &["float", "spectrum"],
        Some(DefaultValue::Floats(&[1.5])),
    ),
    float_ft_default("roughness", &[0.]),
    depends_ft("uroughness", "roughness"),
    depends_ft("vroughness", "roughness"),
    float_ft_default("thickness", &[0.01]),
    integer_row("maxdepth", &[10]),
    integer_row("nsamples", &[1]),
    float_ft_default("g", &[0.]),
    spectrum_st(
        "albedo",
        Some(DefaultValue::Floats(&[0.])),
        TextureInput::SpectrumAlbedo,
    ),
    bool_row("remaproughness", &[true]),
    displacement_row(),
    normalmap_row(),
];

const COATEDCONDUCTOR: &[ParameterSpec] = &[
    float_ft_default("interface.roughness", &[0.]),
    depends_ft("interface.uroughness", "interface.roughness"),
    depends_ft("interface.vroughness", "interface.roughness"),
    value(
        "interface.eta",
        &["float", "spectrum"],
        Some(DefaultValue::Floats(&[1.5])),
    ),
    float_ft_default("thickness", &[0.01]),
    float_ft_default("conductor.roughness", &[0.]),
    depends_ft("conductor.uroughness", "conductor.roughness"),
    depends_ft("conductor.vroughness", "conductor.roughness"),
    spectrum_st(
        "conductor.eta",
        Some(DefaultValue::Strings(&["metal-Cu-eta"])),
        TextureInput::SpectrumUnbounded,
    ),
    spectrum_st(
        "conductor.k",
        Some(DefaultValue::Strings(&["metal-Cu-k"])),
        TextureInput::SpectrumUnbounded,
    ),
    spectrum_st("reflectance", None, TextureInput::SpectrumAlbedo),
    spectrum_st(
        "albedo",
        Some(DefaultValue::Floats(&[0.])),
        TextureInput::SpectrumAlbedo,
    ),
    float_ft_default("g", &[0.]),
    integer_row("maxdepth", &[10]),
    integer_row("nsamples", &[1]),
    bool_row("remaproughness", &[true]),
    displacement_row(),
    normalmap_row(),
];

const DIFFUSETRANSMISSION: &[ParameterSpec] = &[
    ParameterSpec {
        name: "reflectance",
        aliases: &["Kd"],
        value_types: &["spectrum", "rgb"],
        default: Some(DefaultValue::Floats(&[0.25])),
        texture: Some(TextureInput::SpectrumAlbedo),
        role: Role::TextureInput,
        ..EMPTY
    },
    ParameterSpec {
        name: "transmittance",
        aliases: &["Kt"],
        value_types: &["spectrum", "rgb"],
        default: Some(DefaultValue::Floats(&[0.25])),
        texture: Some(TextureInput::SpectrumAlbedo),
        role: Role::TextureInput,
        ..EMPTY
    },
    // Scale is a constant only.
    float_row("scale", &[1.]),
    displacement_row(),
    normalmap_row(),
];

const THINDIELECTRIC: &[ParameterSpec] = &[
    value(
        "eta",
        &["float", "spectrum"],
        Some(DefaultValue::Floats(&[1.5])),
    ),
    displacement_row(),
    normalmap_row(),
];

const HAIR: &[ParameterSpec] = &[
    spectrum_st("sigma_a", None, TextureInput::SpectrumUnbounded),
    spectrum_st("reflectance", None, TextureInput::SpectrumAlbedo),
    spectrum_st("color", None, TextureInput::SpectrumAlbedo),
    float_ft("eumelanin", None),
    float_ft("pheomelanin", None),
    float_ft_default("eta", &[1.55]),
    float_ft_default("beta_m", &[0.3]),
    float_ft_default("beta_n", &[0.3]),
    float_ft_default("alpha", &[2.]),
];

const SUBSURFACE: &[ParameterSpec] = &[
    value("name", &["string"], Some(DefaultValue::Strings(&[""]))),
    spectrum_st("sigma_a", None, TextureInput::SpectrumUnbounded),
    spectrum_st("sigma_s", None, TextureInput::SpectrumUnbounded),
    spectrum_st("reflectance", None, TextureInput::SpectrumAlbedo),
    spectrum_st("mfp", None, TextureInput::SpectrumUnbounded),
    float_row("g", &[0.]),
    float_row("scale", &[1.]),
    float_row("eta", &[1.33]),
    float_ft_default("roughness", &[0.]),
    depends_ft("uroughness", "roughness"),
    depends_ft("vroughness", "roughness"),
    bool_row("remaproughness", &[true]),
    displacement_row(),
    normalmap_row(),
];

const MEASURED: &[ParameterSpec] = &[
    ParameterSpec {
        name: "filename",
        value_types: &["string"],
        role: Role::File,
        required: true,
        ..EMPTY
    },
    displacement_row(),
    normalmap_row(),
];

// Mix has no displacement / normalmap of its own.
const MIX_MATERIAL: &[ParameterSpec] = &[
    ParameterSpec {
        name: "materials",
        aliases: &["namedmaterial1", "namedmaterial2"],
        value_types: &["string"],
        role: Role::Resource,
        required: true,
        ..EMPTY
    },
    float_ft_default("amount", &[0.5]),
];

// ----------------------------------------------------------------- lights

const POINT_LIGHT: &[ParameterSpec] = &[
    illuminant("I"),
    float_row("scale", &[1.]),
    // Negative power means "not given": no photometric normalization.
    float_row("power", &[-1.]),
    value("from", &["point3"], Some(DefaultValue::Floats(FROM))),
];

const SPOT_LIGHT: &[ParameterSpec] = &[
    illuminant("I"),
    float_row("scale", &[1.]),
    float_row("coneangle", &[30.]),
    ParameterSpec {
        name: "conedelta",
        aliases: &["conedeltaangle"],
        value_types: &["float"],
        default: Some(DefaultValue::Floats(&[5.])),
        ..EMPTY
    },
    float_row("power", &[-1.]),
    value("from", &["point3"], Some(DefaultValue::Floats(FROM))),
    value("to", &["point3"], Some(DefaultValue::Floats(TO))),
];

const DISTANT_LIGHT: &[ParameterSpec] = &[
    illuminant("L"),
    float_row("scale", &[1.]),
    float_row("illuminance", &[-1.]),
    value("from", &["point3"], Some(DefaultValue::Floats(FROM))),
    value("to", &["point3"], Some(DefaultValue::Floats(TO))),
];

const INFINITE_LIGHT: &[ParameterSpec] = &[
    illuminant("L"),
    float_row("scale", &[1.]),
    float_row("illuminance", &[-1.]),
    ParameterSpec {
        name: "filename",
        aliases: &["mapname"],
        value_types: &["string"],
        default: Some(DefaultValue::Strings(&[""])),
        role: Role::File,
        ..EMPTY
    },
    // Exactly four points name a portal; otherwise the parameter stays away.
    value("portal", &["point3"], None),
];

// r4's area light has no power, and its twosided defaults to false.
const AREA_DIFFUSE: &[ParameterSpec] = &[
    illuminant("L"),
    float_row("scale", &[1.]),
    bool_row("twosided", &[false]),
    file("filename", &["string"], Some(DefaultValue::Strings(&[""]))),
    integer_row("nsamples", &[1]),
];

// ---------------------------------------------------------------- textures
// The 2D mapping rows ride with their users: imagemap, checkerboard,
// dots and bilerp. The 3D-mapping users take none. r4's UV
// mapping reads the scales and deltas, planar reads v1 / v2 and the
// deltas, spherical and cylindrical read none of them.

const fn mapped(spec: ParameterSpec, when: &'static [Condition]) -> ParameterSpec {
    ParameterSpec {
        group: Some(Group::Mapping),
        when,
        ..spec
    }
}

const fn mapping_is(values: &'static [&'static str]) -> Condition {
    Condition {
        parameter: "mapping",
        values,
    }
}

const MAPPING: ParameterSpec = ParameterSpec {
    choices: &["uv", "spherical", "cylindrical", "planar"],
    ..value("mapping", &["string"], Some(DefaultValue::Strings(&["uv"])))
};
const UV: &[Condition] = &[mapping_is(&["uv"])];
const UV_PLANAR: &[Condition] = &[mapping_is(&["uv", "planar"])];
const PLANAR: &[Condition] = &[mapping_is(&["planar"])];
// Checkerboard maps in 2D only while its dimension is 2.
const DIMENSION_2: Condition = Condition {
    parameter: "dimension",
    values: &["2"],
};
const CHECKER_2D: &[Condition] = &[DIMENSION_2];
const CHECKER_UV: &[Condition] = &[DIMENSION_2, mapping_is(&["uv"])];
const CHECKER_UV_PLANAR: &[Condition] = &[DIMENSION_2, mapping_is(&["uv", "planar"])];
const CHECKER_PLANAR: &[Condition] = &[DIMENSION_2, mapping_is(&["planar"])];

const CONSTANT_F: &[ParameterSpec] = &[value(
    "value",
    &["float"],
    Some(DefaultValue::Floats(&[1.])),
)];
const CONSTANT_S: &[ParameterSpec] = &[value(
    "value",
    &["spectrum", "rgb"],
    Some(DefaultValue::Floats(&[1.])),
)];

// scale (texture): r4 reads the first operand as "tex" (alias tex1) and the
// second as "scale" (alias tex2); both required-none / defaulted as the doc.
const SCALE_F: &[ParameterSpec] = &[
    ParameterSpec {
        name: "tex",
        aliases: &["tex1"],
        value_types: &["float"],
        texture: Some(TextureInput::Float),
        role: Role::TextureInput,
        required: true,
        ..EMPTY
    },
    ParameterSpec {
        name: "scale",
        aliases: &["tex2"],
        value_types: &["float"],
        default: Some(DefaultValue::Floats(&[1.])),
        texture: Some(TextureInput::Float),
        role: Role::TextureInput,
        ..EMPTY
    },
];

const SCALE_S: &[ParameterSpec] = &[
    ParameterSpec {
        name: "tex",
        aliases: &["tex1"],
        value_types: &["spectrum", "rgb"],
        texture: Some(TextureInput::SpectrumAlbedo),
        role: Role::TextureInput,
        required: true,
        ..EMPTY
    },
    ParameterSpec {
        name: "scale",
        aliases: &["tex2"],
        value_types: &["float"],
        default: Some(DefaultValue::Floats(&[1.])),
        texture: Some(TextureInput::Float),
        role: Role::TextureInput,
        ..EMPTY
    },
];

// mix (texture): r4's spectrum tex1 is 1.
const MIX_T_F: &[ParameterSpec] = &[
    tex_operand(
        "tex1",
        &[],
        &["float"],
        Some(DefaultValue::Floats(&[0.])),
        TextureInput::Float,
    ),
    tex_operand(
        "tex2",
        &[],
        &["float"],
        Some(DefaultValue::Floats(&[1.])),
        TextureInput::Float,
    ),
    ParameterSpec {
        name: "amount",
        value_types: &["float"],
        default: Some(DefaultValue::Floats(&[0.5])),
        texture: Some(TextureInput::Float),
        role: Role::TextureInput,
        ..EMPTY
    },
];

const MIX_T_S: &[ParameterSpec] = &[
    tex_operand(
        "tex1",
        &[],
        &["spectrum", "rgb"],
        Some(DefaultValue::Floats(&[1.])),
        TextureInput::SpectrumAlbedo,
    ),
    tex_operand(
        "tex2",
        &[],
        &["spectrum", "rgb"],
        Some(DefaultValue::Floats(&[0.])),
        TextureInput::SpectrumAlbedo,
    ),
    ParameterSpec {
        name: "amount",
        value_types: &["float"],
        default: Some(DefaultValue::Floats(&[0.5])),
        texture: Some(TextureInput::Float),
        role: Role::TextureInput,
        ..EMPTY
    },
];

const DIRECTIONMIX_F: &[ParameterSpec] = &[
    tex_operand(
        "tex1",
        &[],
        &["float"],
        Some(DefaultValue::Floats(&[0.])),
        TextureInput::Float,
    ),
    tex_operand(
        "tex2",
        &[],
        &["float"],
        Some(DefaultValue::Floats(&[1.])),
        TextureInput::Float,
    ),
    value("dir", &["vector"], Some(DefaultValue::Floats(UP))),
];

const DIRECTIONMIX_S: &[ParameterSpec] = &[
    tex_operand(
        "tex1",
        &[],
        &["spectrum", "rgb"],
        Some(DefaultValue::Floats(&[0.])),
        TextureInput::SpectrumAlbedo,
    ),
    tex_operand(
        "tex2",
        &[],
        &["spectrum", "rgb"],
        Some(DefaultValue::Floats(&[1.])),
        TextureInput::SpectrumAlbedo,
    ),
    value("dir", &["vector"], Some(DefaultValue::Floats(UP))),
];

const fn checker_tex(name: &'static str, values: &'static [f64], spectrum: bool) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types: if spectrum {
            &["spectrum", "rgb"]
        } else {
            &["float"]
        },
        default: Some(DefaultValue::Floats(values)),
        texture: Some(if spectrum {
            TextureInput::SpectrumAlbedo
        } else {
            TextureInput::Float
        }),
        role: Role::TextureInput,
        ..EMPTY
    }
}

const fn checkerboard_table(spectrum: bool) -> [ParameterSpec; 11] {
    [
        integer_row("dimension", &[2]),
        checker_tex("tex1", &[1.], spectrum),
        checker_tex("tex2", &[0.], spectrum),
        ParameterSpec {
            choices: &["closedform", "none"],
            ..value(
                "aamode",
                &["string"],
                Some(DefaultValue::Strings(&["closedform"])),
            )
        },
        mapped(MAPPING, CHECKER_2D),
        mapped(float_row("uscale", &[1.]), CHECKER_UV),
        mapped(float_row("vscale", &[1.]), CHECKER_UV),
        mapped(float_row("udelta", &[0.]), CHECKER_UV_PLANAR),
        mapped(float_row("vdelta", &[0.]), CHECKER_UV_PLANAR),
        mapped(
            value("v1", &["vector"], Some(DefaultValue::Floats(PLANAR_U))),
            CHECKER_PLANAR,
        ),
        mapped(
            value("v2", &["vector"], Some(DefaultValue::Floats(PLANAR_V))),
            CHECKER_PLANAR,
        ),
    ]
}

const CHECKERBOARD_F: &[ParameterSpec] = &checkerboard_table(false);
const CHECKERBOARD_S: &[ParameterSpec] = &checkerboard_table(true);

// r4 reads inside (1) and outside (0) as
// pbrt-v4 does, where `inside` shows outside the dots.
const fn dots_table(spectrum: bool) -> [ParameterSpec; 9] {
    [
        checker_tex("inside", &[1.], spectrum),
        checker_tex("outside", &[0.], spectrum),
        mapped(MAPPING, &[]),
        mapped(float_row("uscale", &[1.]), UV),
        mapped(float_row("vscale", &[1.]), UV),
        mapped(float_row("udelta", &[0.]), UV_PLANAR),
        mapped(float_row("vdelta", &[0.]), UV_PLANAR),
        mapped(
            value("v1", &["vector"], Some(DefaultValue::Floats(PLANAR_U))),
            PLANAR,
        ),
        mapped(
            value("v2", &["vector"], Some(DefaultValue::Floats(PLANAR_V))),
            PLANAR,
        ),
    ]
}

const DOTS_F: &[ParameterSpec] = &dots_table(false);
const DOTS_S: &[ParameterSpec] = &dots_table(true);

// fbm and wrinkled share one table; windy has nothing.
const FBM: &[ParameterSpec] = &[integer_row("octaves", &[8]), float_row("roughness", &[0.5])];
const WRINKLED: &[ParameterSpec] = FBM;
const WINDY: &[ParameterSpec] = &[];

// Marble is spectrum-only.
const MARBLE_S: &[ParameterSpec] = &[
    integer_row("octaves", &[8]),
    float_row("roughness", &[0.5]),
    float_row("scale", &[1.]),
    float_row("variation", &[0.2]),
];

// Bilerp takes constants only.
const fn bilerp_corner(
    name: &'static str,
    values: &'static [f64],
    spectrum: bool,
) -> ParameterSpec {
    ParameterSpec {
        name,
        value_types: if spectrum {
            &["spectrum", "rgb"]
        } else {
            &["float"]
        },
        default: Some(DefaultValue::Floats(values)),
        ..EMPTY
    }
}

const fn bilerp_table(spectrum: bool) -> [ParameterSpec; 11] {
    [
        bilerp_corner("v00", &[0.], spectrum),
        bilerp_corner("v01", &[1.], spectrum),
        bilerp_corner("v10", &[0.], spectrum),
        bilerp_corner("v11", &[1.], spectrum),
        mapped(MAPPING, &[]),
        mapped(float_row("uscale", &[1.]), UV),
        mapped(float_row("vscale", &[1.]), UV),
        mapped(float_row("udelta", &[0.]), UV_PLANAR),
        mapped(float_row("vdelta", &[0.]), UV_PLANAR),
        mapped(
            value("v1", &["vector"], Some(DefaultValue::Floats(PLANAR_U))),
            PLANAR,
        ),
        mapped(
            value("v2", &["vector"], Some(DefaultValue::Floats(PLANAR_V))),
            PLANAR,
        ),
    ]
}

const BILERP_F: &[ParameterSpec] = &bilerp_table(false);
const BILERP_S: &[ParameterSpec] = &bilerp_table(true);

const fn imagemap_table() -> [ParameterSpec; 14] {
    [
        ParameterSpec {
            name: "filename",
            value_types: &["string"],
            role: Role::File,
            required: true,
            ..EMPTY
        },
        // r4 also reads "EWA"; the choices keep its canonical spelling.
        ParameterSpec {
            choices: &["point", "bilinear", "trilinear", "ewa"],
            ..value(
                "filter",
                &["string"],
                Some(DefaultValue::Strings(&["bilinear"])),
            )
        },
        float_row("maxanisotropy", &[8.]),
        ParameterSpec {
            name: "wrap",
            aliases: &["swrap", "twrap"],
            value_types: &["string"],
            default: Some(DefaultValue::Strings(&["repeat"])),
            choices: &["repeat", "black", "clamp", "octahedralsphere"],
            ..EMPTY
        },
        float_row("scale", &[1.]),
        bool_row("invert", &[false]),
        // .png decodes as sRGB, the rest as linear.
        ParameterSpec {
            name: "encoding",
            value_types: &["string"],
            // "gamma <value>" takes any number, so these only suggest.
            suggestions: &["sRGB", "linear", "gamma 2.2"],
            default: Some(DefaultValue::Depends {
                parameter: "filename",
                operation: DependsOperation::FileEncoding,
            }),
            ..EMPTY
        },
        mapped(MAPPING, &[]),
        mapped(float_row("uscale", &[1.]), UV),
        mapped(float_row("vscale", &[1.]), UV),
        mapped(float_row("udelta", &[0.]), UV_PLANAR),
        mapped(float_row("vdelta", &[0.]), UV_PLANAR),
        mapped(
            value("v1", &["vector"], Some(DefaultValue::Floats(PLANAR_U))),
            PLANAR,
        ),
        mapped(
            value("v2", &["vector"], Some(DefaultValue::Floats(PLANAR_V))),
            PLANAR,
        ),
    ]
}

const IMAGEMAP_F: &[ParameterSpec] = &imagemap_table();
const IMAGEMAP_S: &[ParameterSpec] = &imagemap_table();

// ------------------------------------------------------------------ medium

// No preset, then r4's named scattering properties (pbrt-r4
// media/medium.rs); r4 warns about any other name and ignores it.
const MEDIUM_PRESETS: &[&str] = &[
    "",
    "Apple",
    "Chicken1",
    "Chicken2",
    "Cream",
    "Ketchup",
    "Marble",
    "Potato",
    "Skimmilk",
    "Skin1",
    "Skin2",
    "Spectralon",
    "Wholemilk",
    "Lowfat Milk",
    "Reduced Milk",
    "Regular Milk",
    "Espresso",
    "Mint Mocha Coffee",
    "Lowfat Soy Milk",
    "Regular Soy Milk",
    "Lowfat Chocolate Milk",
    "Regular Chocolate Milk",
    "Coke",
    "Pepsi",
    "Sprite",
    "Gatorade",
    "Chardonnay",
    "White Zinfandel",
    "Merlot",
    "Budweiser Beer",
    "Coors Light Beer",
    "Clorox",
    "Apple Juice",
    "Cranberry Juice",
    "Grape Juice",
    "Ruby Grapefruit Juice",
    "White Grapefruit Juice",
    "Shampoo",
    "Strawberry Shampoo",
    "Head & Shoulders Shampoo",
    "Lemon Tea Powder",
    "Orange Powder",
    "Pink Lemonade Powder",
    "Cappuccino Powder",
    "Salt Powder",
    "Sugar Powder",
    "Suisse Mocha Powder",
    "Pacific Ocean Surface Water",
];

// r4 prefers explicit sigma over the preset; pbrt-v4 prefers the preset.
const HOMOGENEOUS: &[ParameterSpec] = &[
    ParameterSpec {
        choices: MEDIUM_PRESETS,
        ..value("preset", &["string"], Some(DefaultValue::Strings(&[""])))
    },
    value(
        "sigma_a",
        &["spectrum", "rgb"],
        Some(DefaultValue::PresetElse(&[1.])),
    ),
    value(
        "sigma_s",
        &["spectrum", "rgb"],
        Some(DefaultValue::PresetElse(&[1.])),
    ),
    float_row("scale", &[1.]),
    value(
        "Le",
        &["spectrum", "rgb"],
        Some(DefaultValue::Floats(&[0.])),
    ),
    float_row("Lescale", &[1.]),
    float_row("g", &[0.]),
];

// ------------------------------------------ render configuration
// The options-block directives, transcribed from r4 like the
// rest of this file. Where a shared slice would repeat, the rows are
// spelled per table so each stays one self-contained slice.

/// r4 cameras/perspective.rs:34-64.
const fn perspective_camera_table() -> [ParameterSpec; 8] {
    [
        float_row("shutteropen", &[0.]),
        float_row("shutterclose", &[1.]),
        // r4 derives these: frameaspectratio from the film's aspect ratio
        // (l.47), screenwindow from that frame (l.48-59).
        value("frameaspectratio", &["float"], None),
        value("screenwindow", &["float"], None),
        float_row("lensradius", &[0.]),
        float_row("focaldistance", &[1e6]),
        float_row("fov", &[90.]),
        // A positive halffov overrides fov as twice its value (l.61-64).
        float_row("halffov", &[-1.]),
    ]
}

/// r4 cameras/orthographic.rs:34-78: the shared rows, no fov / halffov.
const fn orthographic_camera_table() -> [ParameterSpec; 6] {
    [
        float_row("shutteropen", &[0.]),
        float_row("shutterclose", &[1.]),
        // Both derive like the perspective camera's (l.51, l.52-78).
        value("frameaspectratio", &["float"], None),
        value("screenwindow", &["float"], None),
        float_row("lensradius", &[0.]),
        float_row("focaldistance", &[1e6]),
    ]
}

const CAMERA_PERSPECTIVE: &[ParameterSpec] = &perspective_camera_table();
const CAMERA_ORTHOGRAPHIC: &[ParameterSpec] = &orthographic_camera_table();

/// The pixel sensor names r4's PixelSensor resolves (film/pixel_sensor.rs:
/// SENSORS plus "cie1931"); anything else errors the render.
const PIXEL_SENSORS: &[&str] = &[
    "cie1931",
    "canon_eos_100d",
    "canon_eos_1dx_mkii",
    "canon_eos_200d",
    "canon_eos_200d_mkii",
    "canon_eos_5d",
    "canon_eos_5d_mkii",
    "canon_eos_5d_mkiii",
    "canon_eos_5d_mkiv",
    "canon_eos_5ds",
    "canon_eos_m",
    "hasselblad_l1d_20c",
    "nikon_d810",
    "nikon_d850",
    "sony_ilce_6400",
    "sony_ilce_7m3",
    "sony_ilce_7rm3",
    "sony_ilce_9",
];

/// r4 base/film.rs:141-177 reads one shared parameter set before the film
/// name; rgb keeps it all. `filename` is the output path the user types —
/// a plain string, never a managed asset; `pixelbounds` (l.116) defaults to
/// the full resolution and `cropwindow` (l.84) to the whole image; the
/// Infinity of `maxcomponentvalue` (l.161) cannot ride this table's finite
/// floats.
const FILM_RGB: &[ParameterSpec] = &[
    value(
        "filename",
        &["string"],
        Some(DefaultValue::Strings(&["pbrt.exr"])),
    ),
    integer_row("xresolution", &[1280]),
    integer_row("yresolution", &[720]),
    value("pixelbounds", &["integer"], None),
    value("cropwindow", &["float"], None),
    float_row("scale", &[1.]),
    float_row("diagonal", &[35.]),
    value("maxcomponentvalue", &["float"], None),
    float_row("iso", &[100.]),
    float_row("whitebalance", &[0.]),
    ParameterSpec {
        choices: PIXEL_SENSORS,
        ..value(
            "sensor",
            &["string"],
            Some(DefaultValue::Strings(&["cie1931"])),
        )
    },
];

/// The sobol family's randomization choices (r4 zsobol.rs:215-224,
/// sobol.rs:191-199, paddedsobol.rs:179-187).
const SOBOL_RANDOMIZATION: &[&str] = &["none", "permutedigits", "fastowen", "owen"];
/// Halton's parser rejects fastowen (r4 halton.rs:257-267).
const HALTON_RANDOMIZATION: &[&str] = &["none", "permutedigits", "owen"];

/// The zsobol family's randomization row (default fastowen).
const fn fastowen_randomization() -> ParameterSpec {
    ParameterSpec {
        choices: SOBOL_RANDOMIZATION,
        ..value(
            "randomization",
            &["string"],
            Some(DefaultValue::Strings(&["fastowen"])),
        )
    }
}

/// Halton's randomization row (permutedigits; its parser rejects fastowen).
const fn permutedigits_randomization() -> ParameterSpec {
    ParameterSpec {
        choices: HALTON_RANDOMIZATION,
        ..value(
            "randomization",
            &["string"],
            Some(DefaultValue::Strings(&["permutedigits"])),
        )
    }
}

/// Every sampler's seed row falls back to the render's global seed, so it
/// carries no constant.
const fn global_seed() -> ParameterSpec {
    value("seed", &["integer"], None)
}

/// r4 samplers/zsobol.rs:129-146 (sobol.rs:131-140 and paddedsobol.rs:115-124
/// read the same with the same defaults).
const SAMPLER_ZSOBOL: &[ParameterSpec] = &[
    integer_row("pixelsamples", &[16]),
    fastowen_randomization(),
    global_seed(),
];
/// r4 samplers/halton.rs:156-164.
const SAMPLER_HALTON: &[ParameterSpec] = &[
    integer_row("pixelsamples", &[16]),
    permutedigits_randomization(),
    global_seed(),
];
const SAMPLER_SOBOL: &[ParameterSpec] = SAMPLER_ZSOBOL;
const SAMPLER_PADDEDSOBOL: &[ParameterSpec] = SAMPLER_ZSOBOL;
/// r4 samplers/pmj02bn.rs:153-155: no randomization row, seed 0.
const SAMPLER_PMJ02BN: &[ParameterSpec] = &[
    integer_row("pixelsamples", &[16]),
    integer_row("seed", &[0]),
];
/// r4 samplers/stratified.rs:124-133.
const SAMPLER_STRATIFIED: &[ParameterSpec] = &[
    bool_row("jitter", &[true]),
    integer_row("xsamples", &[4]),
    integer_row("ysamples", &[4]),
    global_seed(),
];
/// r4 samplers/independent.rs:95-98: pixelsamples 4.
const SAMPLER_INDEPENDENT: &[ParameterSpec] = &[integer_row("pixelsamples", &[4]), global_seed()];

/// The light-sampling strategies r4's LightSampler resolves
/// (base/lightsampler.rs:720-731: r4 adds exhaustive).
const LIGHT_STRATEGIES: &[&str] = &["uniform", "power", "exhaustive", "bvh", "spatial"];

/// r4 cpu/integrators/path.rs:344-361. `lightsampler` (l.354) is checked
/// first — an alternate spelling of the same choice, kept as an alias.
const INTEGRATOR_PATH: &[ParameterSpec] = &[
    integer_row("maxdepth", &[5]),
    ParameterSpec {
        aliases: &["lightsampler"],
        choices: LIGHT_STRATEGIES,
        ..value(
            "lightsamplestrategy",
            &["string"],
            Some(DefaultValue::Strings(&["bvh"])),
        )
    },
    bool_row("regularize", &[false]),
];
/// r4 cpu/integrators/volpath.rs:877-894 reads the same with the same
/// defaults.
const INTEGRATOR_VOLPATH: &[ParameterSpec] = INTEGRATOR_PATH;

/// r4 filters/box_filter.rs:19-22.
const FILTER_BOX: &[ParameterSpec] = &[float_row("xradius", &[0.5]), float_row("yradius", &[0.5])];
/// r4 filters/gaussian.rs:37-41.
const FILTER_GAUSSIAN: &[ParameterSpec] = &[
    float_row("xradius", &[1.5]),
    float_row("yradius", &[1.5]),
    float_row("sigma", &[0.5]),
];
/// r4 filters/mitchell.rs:31-36.
const FILTER_MITCHELL: &[ParameterSpec] = &[
    float_row("xradius", &[2.]),
    float_row("yradius", &[2.]),
    float_row("B", &[1. / 3.]),
    float_row("C", &[1. / 3.]),
];
/// r4 filters/sinc.rs:29-33.
const FILTER_SINC: &[ParameterSpec] = &[
    float_row("xradius", &[4.]),
    float_row("yradius", &[4.]),
    float_row("tau", &[3.]),
];
/// r4 filters/triangle.rs:21-24.
const FILTER_TRIANGLE: &[ParameterSpec] =
    &[float_row("xradius", &[2.]), float_row("yradius", &[2.])];

/// r4 cpu/aggregates/bvh/create_bvh_accelerator.rs:10,31,45: the default
/// split method is hlbvh (where pbrt-v4 reads sah), and a spelling outside
/// the choices silently falls back to SAH (l.36-41).
const ACCELERATOR_BVH: &[ParameterSpec] = &[
    ParameterSpec {
        choices: &["sah", "hlbvh", "middle", "equal"],
        ..value(
            "splitmethod",
            &["string"],
            Some(DefaultValue::Strings(&["hlbvh"])),
        )
    },
    integer_row("maxnodeprims", &[8]),
];
/// r4 cpu/aggregates/kdtree/accel.rs:521-527.
const ACCELERATOR_KDTREE: &[ParameterSpec] = &[
    integer_row("intersectcost", &[80]),
    integer_row("traversalcost", &[1]),
    float_row("emptybonus", &[0.5]),
    integer_row("maxprims", &[1]),
    integer_row("maxdepth", &[-1]),
];

// ------------------------------------------------------------ the tables

/// The parameter tables of one implementation. Textures have one table per
/// texture value type (the doc's float / spectrum columns); marble is
/// spectrum-only.
pub enum Tables {
    Fixed(&'static [ParameterSpec]),
    Texture {
        float: Option<&'static [ParameterSpec]>,
        spectrum: Option<&'static [ParameterSpec]>,
    },
}

/// One implementation: canonical name, r4 aliases, parameter tables and the
/// doc's mutually exclusive parameter groups.
pub struct ImplementationSchema {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub tables: Tables,
    pub exclusive: &'static [&'static [&'static str]],
}

const fn fixed(
    name: &'static str,
    aliases: &'static [&'static str],
    parameters: &'static [ParameterSpec],
) -> ImplementationSchema {
    ImplementationSchema {
        name,
        aliases,
        tables: Tables::Fixed(parameters),
        exclusive: &[],
    }
}

const fn texture_impl(
    name: &'static str,
    float: Option<&'static [ParameterSpec]>,
    spectrum: Option<&'static [ParameterSpec]>,
) -> ImplementationSchema {
    ImplementationSchema {
        name,
        aliases: &[],
        tables: Tables::Texture { float, spectrum },
        exclusive: &[],
    }
}

const SHAPES: &[ImplementationSchema] = &[
    fixed("sphere", &[], SPHERE),
    fixed("disk", &[], DISK),
    fixed("cylinder", &[], CYLINDER),
    fixed("trianglemesh", &[], TRIANGLEMESH),
    fixed("bilinearmesh", &[], BILINEARMESH),
    fixed("plymesh", &[], PLYMESH),
];

const MATERIALS: &[ImplementationSchema] = &[
    fixed("diffuse", &[], DIFFUSE),
    fixed("thindielectric", &[], THINDIELECTRIC),
    fixed("hair", &[], HAIR),
    fixed("subsurface", &[], SUBSURFACE),
    fixed("measured", &[], MEASURED),
    ImplementationSchema {
        name: "conductor",
        aliases: &[],
        tables: Tables::Fixed(CONDUCTOR),
        exclusive: &[&["eta", "k"], &["reflectance"]],
    },
    fixed("dielectric", &[], DIELECTRIC),
    fixed("coateddiffuse", &[], COATEDDIFFUSE),
    ImplementationSchema {
        name: "coatedconductor",
        aliases: &[],
        tables: Tables::Fixed(COATEDCONDUCTOR),
        exclusive: &[&["conductor.eta", "conductor.k"], &["reflectance"]],
    },
    fixed("diffusetransmission", &[], DIFFUSETRANSMISSION),
    fixed("mix", &[], MIX_MATERIAL),
];

const LIGHTS: &[ImplementationSchema] = &[
    fixed("point", &[], POINT_LIGHT),
    fixed("spot", &[], SPOT_LIGHT),
    fixed("distant", &[], DISTANT_LIGHT),
    fixed("infinite", &[], INFINITE_LIGHT),
];

const AREA_LIGHTS: &[ImplementationSchema] = &[ImplementationSchema {
    name: "diffuse",
    aliases: &["area", ""],
    tables: Tables::Fixed(AREA_DIFFUSE),
    exclusive: &[],
}];

const TEXTURES: &[ImplementationSchema] = &[
    texture_impl("constant", Some(CONSTANT_F), Some(CONSTANT_S)),
    texture_impl("imagemap", Some(IMAGEMAP_F), Some(IMAGEMAP_S)),
    texture_impl("scale", Some(SCALE_F), Some(SCALE_S)),
    texture_impl("mix", Some(MIX_T_F), Some(MIX_T_S)),
    texture_impl("directionmix", Some(DIRECTIONMIX_F), Some(DIRECTIONMIX_S)),
    texture_impl("checkerboard", Some(CHECKERBOARD_F), Some(CHECKERBOARD_S)),
    texture_impl("dots", Some(DOTS_F), Some(DOTS_S)),
    texture_impl("fbm", Some(FBM), Some(FBM)),
    texture_impl("wrinkled", Some(WRINKLED), Some(WRINKLED)),
    texture_impl("windy", Some(WINDY), Some(WINDY)),
    texture_impl("marble", None, Some(MARBLE_S)),
    texture_impl("bilerp", Some(BILERP_F), Some(BILERP_S)),
];

const MEDIA: &[ImplementationSchema] = &[fixed("homogeneous", &[], HOMOGENEOUS)];

// These implementations. The tables stay transcriptions: callers
// serve what is here and keep serving implementations without one.
const CAMERAS: &[ImplementationSchema] = &[
    fixed("perspective", &[], CAMERA_PERSPECTIVE),
    fixed("orthographic", &[], CAMERA_ORTHOGRAPHIC),
];

const FILMS: &[ImplementationSchema] = &[fixed("rgb", &[], FILM_RGB)];

const SAMPLERS: &[ImplementationSchema] = &[
    fixed("zsobol", &[], SAMPLER_ZSOBOL),
    fixed("halton", &[], SAMPLER_HALTON),
    fixed("sobol", &[], SAMPLER_SOBOL),
    fixed("paddedsobol", &[], SAMPLER_PADDEDSOBOL),
    fixed("pmj02bn", &[], SAMPLER_PMJ02BN),
    fixed("stratified", &[], SAMPLER_STRATIFIED),
    fixed("independent", &[], SAMPLER_INDEPENDENT),
];

const INTEGRATORS: &[ImplementationSchema] = &[
    fixed("path", &[], INTEGRATOR_PATH),
    fixed("volpath", &[], INTEGRATOR_VOLPATH),
];

const PIXEL_FILTERS: &[ImplementationSchema] = &[
    fixed("box", &[], FILTER_BOX),
    fixed("gaussian", &[], FILTER_GAUSSIAN),
    fixed("mitchell", &[], FILTER_MITCHELL),
    fixed("sinc", &[], FILTER_SINC),
    fixed("triangle", &[], FILTER_TRIANGLE),
];

const ACCELERATORS: &[ImplementationSchema] = &[
    fixed("bvh", &[], ACCELERATOR_BVH),
    fixed("kdtree", &[], ACCELERATOR_KDTREE),
];

/// The SpectrumData resource's sample tables (its `piecewise`
/// implementation): the two float arrays the curve holds. They
/// let the Inspector list the resource's own rows — the array editor
/// reads them, the Spectrum Data editor edits them — instead of the
/// "not in this implementation's parameter table" note.
const PIECEWISE: &[ParameterSpec] = &[
    required("wavelengths", &["float"]),
    required("values", &["float"]),
];

const SPECTRUM_DATA: &[ImplementationSchema] = &[fixed("piecewise", &[], PIECEWISE)];

/// The table for one PBRT directive. `Material` and `MakeNamedMaterial`
/// share one table (the same rows either way); None for directives the
/// first edition does not cover.
pub fn schema(directive: &str) -> Option<&'static [ImplementationSchema]> {
    match directive {
        "Shape" => Some(SHAPES),
        "Material" | "MakeNamedMaterial" => Some(MATERIALS),
        "LightSource" => Some(LIGHTS),
        "AreaLightSource" => Some(AREA_LIGHTS),
        "Texture" => Some(TEXTURES),
        "MakeNamedMedium" => Some(MEDIA),
        "Camera" => Some(CAMERAS),
        "Film" => Some(FILMS),
        "Sampler" => Some(SAMPLERS),
        "Integrator" => Some(INTEGRATORS),
        "PixelFilter" => Some(PIXEL_FILTERS),
        "Accelerator" => Some(ACCELERATORS),
        "SpectrumData" => Some(SPECTRUM_DATA),
        _ => None,
    }
}

/// Every PBRT directive the dependency schema knows, table or not; callers
/// serve `parameters: null` with an empty implementation list for the rest.
pub fn is_directive(directive: &str) -> bool {
    matches!(
        directive,
        "Shape"
            | "Material"
            | "MakeNamedMaterial"
            | "Texture"
            | "MakeNamedMedium"
            | "LightSource"
            | "AreaLightSource"
            | "Camera"
            | "Film"
            | "Sampler"
            | "Integrator"
            | "PixelFilter"
            | "Accelerator"
            | "SpectrumData"
    )
}

/// Resolve one implementation by canonical name or r4 alias.
pub fn implementation<'a>(
    table: &'a [ImplementationSchema],
    name: &str,
) -> Option<&'a ImplementationSchema> {
    table.iter().find(|implementation| {
        implementation.name == name || implementation.aliases.contains(&name)
    })
}

impl ImplementationSchema {
    /// The parameter rows for this implementation; for textures, the rows of
    /// one texture value type. None when there is no table (any
    /// implementation without one, a texture without that value type, or a
    /// type passed for a non-texture implementation).
    pub fn parameters(
        &self,
        texture_value_type: Option<TextureValueType>,
    ) -> Option<&'static [ParameterSpec]> {
        match &self.tables {
            Tables::Fixed(parameters) => texture_value_type.is_none().then_some(*parameters),
            Tables::Texture { float, spectrum } => match texture_value_type {
                None => None,
                Some(TextureValueType::Float) => *float,
                Some(TextureValueType::Spectrum) => *spectrum,
            },
        }
    }
}

/// The parameters a newly created item declares:
/// every constant default of the table, minus the ones today's export rules
/// cannot carry — plain strings outside the dependency whitelist — and
/// empty strings (file names, the medium preset), which mean "absent". r4
/// applies the same values when those parameters are simply not written.
pub fn declarable_defaults(
    directive: &str,
    implementation_name: &str,
    texture_value_type: Option<TextureValueType>,
) -> Vec<Parameter> {
    schema(directive)
        .and_then(|table| implementation(table, implementation_name))
        .and_then(|found| found.parameters(texture_value_type))
        .into_iter()
        .flatten()
        .filter_map(|spec| {
            let parameter = spec.default_parameter()?;
            let exportable = match &parameter.values {
                project::ParameterValues::Strings(values) => {
                    !values.iter().any(String::is_empty)
                        && (crate::schema::input_file(directive, implementation_name, spec.name)
                            || crate::schema::plain_string(directive, spec.name))
                }
                _ => true,
            };
            exportable.then_some(parameter)
        })
        .collect()
}

/// The parameters a newly added area light component declares: the table's
/// constant defaults plus the documented
/// `L [1 1 1]`, which the table itself keeps as the scene's illuminant
/// default — `declarable_defaults` alone would declare nothing to edit.
/// An unknown implementation declares only L.
pub fn initial_area_light(implementation: &str) -> Vec<project::Parameter> {
    // The explicit "no area light" carries no parameters: it cancels, it
    // does not declare.
    if implementation.trim().is_empty() {
        return Vec::new();
    }
    // Spectral by nature: the table accepts spectrum/rgb/blackbody for L,
    // so the row is rgb with three floats like every other color row.
    let mut parameters = vec![project::Parameter {
        name: "L".into(),
        value_type: "rgb".into(),
        color_space: project::ColorSpaceBinding::Inherit,
        values: project::ParameterValues::Floats(
            [1., 1., 1.].map(project::FloatValue::Finite).to_vec(),
        ),
    }];
    parameters.extend(declarable_defaults("AreaLightSource", implementation, None));
    parameters
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Value => "value",
            Role::TextureInput => "texture_input",
            Role::File => "file",
            Role::Resource => "resource",
        }
    }
}

impl TextureInput {
    pub fn as_str(self) -> &'static str {
        match self {
            TextureInput::Float => "float",
            TextureInput::SpectrumAlbedo => "spectrum_albedo",
            TextureInput::SpectrumUnbounded => "spectrum_unbounded",
            TextureInput::SpectrumIlluminant => "spectrum_illuminant",
        }
    }
}

impl DependsOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            DependsOperation::Same => "same",
            DependsOperation::Negated => "negated",
            DependsOperation::SequentialIndices => "sequential_indices",
            DependsOperation::FileEncoding => "file_encoding",
        }
    }
}

impl ParameterSpec {
    /// The default as a real `Parameter`, for defaults that are constants
    /// r4 accepts as written. `None` for absent defaults and the ones r4
    /// resolves at creation (illuminants, depends, presets).
    ///
    /// A single float of a spectrum parameter becomes an `rgb` triple (one
    /// `spectrum` scalars would be wavelength pairs); builtin spectrum
    /// names stay names.
    pub fn default_parameter(&self) -> Option<Parameter> {
        let default = self.default.as_ref()?;
        let (value_type, values) = match default {
            DefaultValue::Floats(values) => {
                let value_type = ["point3", "point2", "normal", "vector"]
                    .iter()
                    .find(|kind| self.value_types.contains(kind))
                    .copied()
                    .unwrap_or(
                        if self
                            .value_types
                            .iter()
                            .any(|kind| matches!(*kind, "spectrum" | "rgb" | "blackbody"))
                        {
                            "rgb"
                        } else {
                            "float"
                        },
                    );
                let values: Vec<f64> = if value_type == "rgb" && values.len() == 1 {
                    std::iter::repeat_n(values[0], 3).collect()
                } else {
                    values.to_vec()
                };
                (
                    value_type.into(),
                    project::ParameterValues::Floats(
                        values
                            .into_iter()
                            .map(project::FloatValue::Finite)
                            .collect(),
                    ),
                )
            }
            DefaultValue::Integers(values) => (
                "integer".into(),
                project::ParameterValues::Integers(values.to_vec()),
            ),
            DefaultValue::Booleans(values) => (
                "bool".into(),
                project::ParameterValues::Booleans(values.to_vec()),
            ),
            DefaultValue::Strings(values) if self.value_types.contains(&"string") => (
                "string".into(),
                project::ParameterValues::Strings(
                    values.iter().copied().map(str::to_owned).collect(),
                ),
            ),
            // Otherwise these string values are builtin spectrum names.
            DefaultValue::Strings(values) => (
                "spectrum".into(),
                project::ParameterValues::BuiltinSpectra(
                    values.iter().copied().map(str::to_owned).collect(),
                ),
            ),
            DefaultValue::Illuminant
            | DefaultValue::Depends { .. }
            | DefaultValue::PresetElse(_) => return None,
        };
        Some(Parameter {
            name: self.name.into(),
            value_type,
            color_space: project::ColorSpaceBinding::Inherit,
            values,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One implementation's rows; None until its table exists.
    fn rows(directive: &str, name: &str) -> Option<Vec<&'static ParameterSpec>> {
        schema(directive)
            .and_then(|table| implementation(table, name))
            .and_then(|found| found.parameters(None))
            .map(|parameters| parameters.iter().collect())
    }

    /// The SpectrumData resource's own two sample tables — the
    /// Inspector's parameter rows the way every other resource has them
    /// (no more "not in this implementation's parameter table"), so the
    /// `piecewise` resource lists its wavelength and value tables like
    /// any other implementation lists its parameters.
    #[test]
    fn spectrum_data_lists_its_two_sample_tables() {
        assert!(is_directive("SpectrumData"));
        let rows = rows("SpectrumData", "piecewise").expect("the piecewise table");
        let names =
            |rows: &[&'static ParameterSpec]| rows.iter().map(|spec| spec.name).collect::<Vec<_>>();
        assert_eq!(names(&rows), ["wavelengths", "values"]);
        for spec in &rows {
            assert_eq!(
                spec.value_types,
                ["float"],
                "{} takes the two float arrays",
                spec.name
            );
            assert!(spec.required, "{} is the curve itself", spec.name);
        }
        // The only implementation of the kind.
        assert_eq!(
            crate::implementation_change::candidates("SpectrumData"),
            ["piecewise"]
        );
    }

    fn default<'a>(rows: &'a [&'static ParameterSpec], name: &str) -> &'a Option<DefaultValue> {
        &rows
            .iter()
            .find(|spec| spec.name == name || spec.aliases.contains(&name))
            .unwrap_or_else(|| panic!("no row {name}"))
            .default
    }

    fn spec<'a>(rows: &'a [&'static ParameterSpec], name: &str) -> &'a ParameterSpec {
        rows.iter()
            .find(|spec| spec.name == name)
            .unwrap_or_else(|| panic!("no row {name}"))
    }

    /// The tables stay transcriptions: rows appear in r4's read order.
    fn names(directive: &str, name: &str) -> Vec<&'static str> {
        rows(directive, name)
            .unwrap()
            .iter()
            .map(|spec| spec.name)
            .collect()
    }

    #[test]
    fn schema_covers_the_render_configuration_directives() {
        for directive in [
            "Camera",
            "Film",
            "Sampler",
            "Integrator",
            "PixelFilter",
            "Accelerator",
        ] {
            assert!(schema(directive).is_some(), "{directive}");
            assert!(is_directive(directive));
        }
        // Unknown directives and the out-of-table Option entity stay None.
        assert!(schema("Option").is_none());
        assert!(schema("ColorSpace").is_none());
        assert!(!is_directive("Option"));
        assert!(!is_directive("ColorSpace"));
    }

    #[test]
    fn camera_perspective_defaults_match_r4() {
        // r4 cameras/perspective.rs:34-64.
        let rows = rows("Camera", "perspective").expect("a table");
        assert_eq!(
            names("Camera", "perspective"),
            [
                "shutteropen",
                "shutterclose",
                "frameaspectratio",
                "screenwindow",
                "lensradius",
                "focaldistance",
                "fov",
                "halffov",
            ]
        );
        assert_eq!(
            default(&rows, "shutteropen"),
            &Some(DefaultValue::Floats(&[0.]))
        );
        assert_eq!(
            default(&rows, "shutterclose"),
            &Some(DefaultValue::Floats(&[1.]))
        );
        // r4 derives both from the film: frameaspectratio from the film's
        // aspect ratio (l.47), screenwindow from frame (l.48-59).
        assert_eq!(default(&rows, "frameaspectratio"), &None);
        assert_eq!(default(&rows, "screenwindow"), &None);
        assert_eq!(
            default(&rows, "lensradius"),
            &Some(DefaultValue::Floats(&[0.]))
        );
        assert_eq!(
            default(&rows, "focaldistance"),
            &Some(DefaultValue::Floats(&[1e6]))
        );
        assert_eq!(default(&rows, "fov"), &Some(DefaultValue::Floats(&[90.])));
        // r4 reads halffov at -1; a positive value overrides fov (l.61-64).
        assert_eq!(
            default(&rows, "halffov"),
            &Some(DefaultValue::Floats(&[-1.]))
        );
        for name in [
            "shutteropen",
            "shutterclose",
            "frameaspectratio",
            "screenwindow",
            "lensradius",
            "focaldistance",
            "fov",
            "halffov",
        ] {
            assert_eq!(spec(&rows, name).value_types, ["float"], "{name}");
        }
    }

    #[test]
    fn camera_orthographic_defaults_match_r4() {
        // r4 cameras/orthographic.rs:34-78: the shared rows, no fov/halffov.
        let rows = rows("Camera", "orthographic").expect("a table");
        assert_eq!(
            names("Camera", "orthographic"),
            [
                "shutteropen",
                "shutterclose",
                "frameaspectratio",
                "screenwindow",
                "lensradius",
                "focaldistance",
            ]
        );
        assert_eq!(
            default(&rows, "shutteropen"),
            &Some(DefaultValue::Floats(&[0.]))
        );
        assert_eq!(
            default(&rows, "shutterclose"),
            &Some(DefaultValue::Floats(&[1.]))
        );
        assert_eq!(default(&rows, "frameaspectratio"), &None);
        assert_eq!(default(&rows, "screenwindow"), &None);
        assert_eq!(
            default(&rows, "lensradius"),
            &Some(DefaultValue::Floats(&[0.]))
        );
        assert_eq!(
            default(&rows, "focaldistance"),
            &Some(DefaultValue::Floats(&[1e6]))
        );
    }

    /// The film camera candidates currently listed, in the table's order.
    #[test]
    fn camera_implementations_are_the_perspective_orthographic_table() {
        let table = schema("Camera").expect("a table");
        let listed: Vec<&str> = table.iter().map(|item| item.name).collect();
        assert_eq!(listed, ["perspective", "orthographic"]);
        // The other exportable camera stays legal to use (implementation
        // names normalize only when the table knows them).
        assert!(crate::schema::supports("Camera", "realistic"));
        assert!(implementation(table, "realistic").is_none());
    }

    #[test]
    fn film_rgb_defaults_match_r4() {
        // r4 base/film.rs:141-177; the pixel bounds and crop window come
        // from get_pixel_bounds_from_params (l.110-129: pixelbounds as four
        // integers over the full resolution) and get_crop_window (l.83-106).
        let rows = rows("Film", "rgb").expect("a table");
        assert_eq!(
            names("Film", "rgb"),
            [
                "filename",
                "xresolution",
                "yresolution",
                "pixelbounds",
                "cropwindow",
                "scale",
                "diagonal",
                "maxcomponentvalue",
                "iso",
                "whitebalance",
                "sensor",
            ]
        );
        // The output name stays a plain string the user types, not a
        // managed asset (the doc's file role).
        assert_eq!(
            default(&rows, "filename"),
            &Some(DefaultValue::Strings(&["pbrt.exr"]))
        );
        assert_eq!(spec(&rows, "filename").value_types, ["string"]);
        assert_eq!(spec(&rows, "filename").role, Role::Value);
        assert_eq!(
            default(&rows, "xresolution"),
            &Some(DefaultValue::Integers(&[1280]))
        );
        assert_eq!(
            default(&rows, "yresolution"),
            &Some(DefaultValue::Integers(&[720]))
        );
        // Derived: pixelbounds from the full resolution (l.116), cropwindow
        // empty (l.84) — the whole image.
        assert_eq!(default(&rows, "pixelbounds"), &None);
        assert_eq!(spec(&rows, "pixelbounds").value_types, ["integer"]);
        assert_eq!(default(&rows, "cropwindow"), &None);
        assert_eq!(default(&rows, "scale"), &Some(DefaultValue::Floats(&[1.])));
        assert_eq!(
            default(&rows, "diagonal"),
            &Some(DefaultValue::Floats(&[35.]))
        );
        // r4's default is Infinity (l.161), which cannot ride the table's
        // finite floats.
        assert_eq!(default(&rows, "maxcomponentvalue"), &None);
        assert_eq!(default(&rows, "iso"), &Some(DefaultValue::Floats(&[100.])));
        assert_eq!(
            default(&rows, "whitebalance"),
            &Some(DefaultValue::Floats(&[0.]))
        );
        assert_eq!(
            default(&rows, "sensor"),
            &Some(DefaultValue::Strings(&["cie1931"]))
        );
        // r4's PixelSensor knows exactly these names (pixel_sensor.rs)
        // plus cie1931; an unknown sensor errors the render.
        assert_eq!(
            spec(&rows, "sensor").choices,
            [
                "cie1931",
                "canon_eos_100d",
                "canon_eos_1dx_mkii",
                "canon_eos_200d",
                "canon_eos_200d_mkii",
                "canon_eos_5d",
                "canon_eos_5d_mkii",
                "canon_eos_5d_mkiii",
                "canon_eos_5d_mkiv",
                "canon_eos_5ds",
                "canon_eos_m",
                "hasselblad_l1d_20c",
                "nikon_d810",
                "nikon_d850",
                "sony_ilce_6400",
                "sony_ilce_7m3",
                "sony_ilce_7rm3",
                "sony_ilce_9",
            ]
        );
    }

    #[test]
    fn film_implementations_are_the_rgb_table() {
        let table = schema("Film").expect("a table");
        let listed: Vec<&str> = table.iter().map(|item| item.name).collect();
        assert_eq!(listed, ["rgb"]);
        assert!(crate::schema::supports("Film", "gbuffer"));
        assert!(implementation(table, "gbuffer").is_none());
    }

    /// What the sobol-style samplers share: pixelsamples 16, one
    /// randomization row, and a seed that follows the render's global seed.
    #[test]
    fn sampler_defaults_match_r4() {
        for name in ["zsobol", "sobol", "paddedsobol"] {
            let rows = rows("Sampler", name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(
                default(&rows, "pixelsamples"),
                &Some(DefaultValue::Integers(&[16])),
                "{name}"
            );
            assert_eq!(
                default(&rows, "randomization"),
                &Some(DefaultValue::Strings(&["fastowen"])),
                "{name}"
            );
            assert_eq!(spec(&rows, "randomization").choices.len(), 4, "{name}");
            for value in ["none", "permutedigits", "fastowen", "owen"] {
                assert!(
                    spec(&rows, "randomization").choices.contains(&value),
                    "{name}: {value}"
                );
            }
            assert_eq!(default(&rows, "seed"), &None, "{name}");
        }
        // r4 samplers/halton.rs:156-164 defaults permutedigits and rejects
        // fastowen (parse l.257-267).
        let halton = rows("Sampler", "halton").expect("halton");
        assert_eq!(
            default(&halton, "randomization"),
            &Some(DefaultValue::Strings(&["permutedigits"]))
        );
        assert!(!spec(&halton, "randomization").choices.contains(&"fastowen"));
        assert_eq!(default(&halton, "seed"), &None);
        // r4 samplers/pmj02bn.rs:153-155: no randomization row, seed 0.
        let pmj02bn = rows("Sampler", "pmj02bn").expect("pmj02bn");
        assert_eq!(
            default(&pmj02bn, "pixelsamples"),
            &Some(DefaultValue::Integers(&[16]))
        );
        assert_eq!(
            sort(pmj02bn.iter().map(|spec| spec.name)),
            ["pixelsamples", "seed"]
        );
        assert_eq!(
            default(&pmj02bn, "seed"),
            &Some(DefaultValue::Integers(&[0]))
        );
        // r4 samplers/stratified.rs:125-128.
        let stratified = rows("Sampler", "stratified").expect("stratified");
        assert_eq!(
            default(&stratified, "jitter"),
            &Some(DefaultValue::Booleans(&[true]))
        );
        assert_eq!(
            default(&stratified, "xsamples"),
            &Some(DefaultValue::Integers(&[4]))
        );
        assert_eq!(
            default(&stratified, "ysamples"),
            &Some(DefaultValue::Integers(&[4]))
        );
        assert_eq!(default(&stratified, "seed"), &None);
        assert_eq!(
            sort(stratified.iter().map(|spec| spec.name)),
            ["jitter", "seed", "xsamples", "ysamples"]
        );
        // r4 samplers/independent.rs:96-97: pixelsamples 4.
        let independent = rows("Sampler", "independent").expect("independent");
        assert_eq!(
            default(&independent, "pixelsamples"),
            &Some(DefaultValue::Integers(&[4]))
        );
        assert_eq!(default(&independent, "seed"), &None);
        // The seed rows whose default follows the render's global seed
        // (r4 PbrtOptions.seed, options/pbrt_options.rs:71) carry no
        // constant; zsobol's is the sample for the doc comment.
        for name in [
            "zsobol",
            "halton",
            "sobol",
            "paddedsobol",
            "stratified",
            "independent",
        ] {
            let rows = rows("Sampler", name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(spec(&rows, "seed").value_types, ["integer"], "{name}");
        }
    }

    #[test]
    fn sampler_implementations_match_r4_names() {
        let table = schema("Sampler").expect("a table");
        let listed: Vec<&str> = table.iter().map(|item| item.name).collect();
        assert_eq!(
            sort(listed.iter().copied()),
            [
                "halton",
                "independent",
                "paddedsobol",
                "pmj02bn",
                "sobol",
                "stratified",
                "zsobol",
            ]
        );
    }

    #[test]
    fn integrator_defaults_match_r4() {
        // r4 cpu/integrators/path.rs:344-361 and volpath.rs:877-894: the
        // same rows and defaults.
        for name in ["path", "volpath"] {
            let rows = rows("Integrator", name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(
                names("Integrator", name),
                ["maxdepth", "lightsamplestrategy", "regularize"],
                "{name}"
            );
            assert_eq!(
                default(&rows, "maxdepth"),
                &Some(DefaultValue::Integers(&[5])),
                "{name}"
            );
            assert_eq!(
                default(&rows, "regularize"),
                &Some(DefaultValue::Booleans(&[false])),
                "{name}"
            );
            let lightsamplestrategy = spec(&rows, "lightsamplestrategy");
            assert_eq!(
                lightsamplestrategy.default,
                Some(DefaultValue::Strings(&["bvh"]))
            );
            // r4 reads "lightsampler" first (path.rs:354) — an alternate
            // spelling of the same choice.
            assert_eq!(lightsamplestrategy.aliases, ["lightsampler"]);
            // r4 base/lightsampler.rs:720-731.
            assert_eq!(
                lightsamplestrategy.choices,
                ["uniform", "power", "exhaustive", "bvh", "spatial"]
            );
        }
    }

    #[test]
    fn integrator_implementations_are_the_path_volpath_table() {
        let table = schema("Integrator").expect("a table");
        assert_eq!(
            table.iter().map(|item| item.name).collect::<Vec<_>>(),
            ["path", "volpath"]
        );
        assert!(crate::schema::supports("Integrator", "bdpt"));
    }

    #[test]
    fn pixel_filter_defaults_match_r4() {
        // r4 filters: box_filter.rs:19-22, gaussian.rs:37-41,
        // mitchell.rs:31-36, sinc.rs:29-33, triangle.rs:21-24.
        let box_filter = rows("PixelFilter", "box").expect("box");
        assert_eq!(names("PixelFilter", "box"), ["xradius", "yradius"]);
        assert_eq!(
            default(&box_filter, "xradius"),
            &Some(DefaultValue::Floats(&[0.5]))
        );
        assert_eq!(
            default(&box_filter, "yradius"),
            &Some(DefaultValue::Floats(&[0.5]))
        );
        let gaussian = rows("PixelFilter", "gaussian").expect("gaussian");
        assert_eq!(
            names("PixelFilter", "gaussian"),
            ["xradius", "yradius", "sigma"]
        );
        assert_eq!(
            default(&gaussian, "xradius"),
            &Some(DefaultValue::Floats(&[1.5]))
        );
        assert_eq!(
            default(&gaussian, "yradius"),
            &Some(DefaultValue::Floats(&[1.5]))
        );
        assert_eq!(
            default(&gaussian, "sigma"),
            &Some(DefaultValue::Floats(&[0.5]))
        );
        let mitchell = rows("PixelFilter", "mitchell").expect("mitchell");
        assert_eq!(
            names("PixelFilter", "mitchell"),
            ["xradius", "yradius", "B", "C"]
        );
        assert_eq!(
            default(&mitchell, "xradius"),
            &Some(DefaultValue::Floats(&[2.]))
        );
        assert_eq!(
            default(&mitchell, "yradius"),
            &Some(DefaultValue::Floats(&[2.]))
        );
        assert_eq!(
            default(&mitchell, "B"),
            &Some(DefaultValue::Floats(&[1. / 3.]))
        );
        assert_eq!(
            default(&mitchell, "C"),
            &Some(DefaultValue::Floats(&[1. / 3.]))
        );
        let sinc = rows("PixelFilter", "sinc").expect("sinc");
        assert_eq!(names("PixelFilter", "sinc"), ["xradius", "yradius", "tau"]);
        assert_eq!(
            default(&sinc, "xradius"),
            &Some(DefaultValue::Floats(&[4.]))
        );
        assert_eq!(
            default(&sinc, "yradius"),
            &Some(DefaultValue::Floats(&[4.]))
        );
        assert_eq!(default(&sinc, "tau"), &Some(DefaultValue::Floats(&[3.])));
        let triangle = rows("PixelFilter", "triangle").expect("triangle");
        assert_eq!(names("PixelFilter", "triangle"), ["xradius", "yradius"]);
        assert_eq!(
            default(&triangle, "xradius"),
            &Some(DefaultValue::Floats(&[2.]))
        );
        assert_eq!(
            default(&triangle, "yradius"),
            &Some(DefaultValue::Floats(&[2.]))
        );
        let table = schema("PixelFilter").expect("a table");
        assert_eq!(
            table.iter().map(|item| item.name).collect::<Vec<_>>(),
            ["box", "gaussian", "mitchell", "sinc", "triangle"]
        );
    }

    #[test]
    fn accelerator_defaults_match_r4() {
        // r4 cpu/aggregates/bvh/create_bvh_accelerator.rs:10,31,45 — the
        // default split method is hlbvh, where pbrt-v4 reads sah; an
        // unknown spelling falls back to SAH (l.36-41).
        let bvh = rows("Accelerator", "bvh").expect("bvh");
        assert_eq!(names("Accelerator", "bvh"), ["splitmethod", "maxnodeprims"]);
        assert_eq!(
            default(&bvh, "splitmethod"),
            &Some(DefaultValue::Strings(&["hlbvh"]))
        );
        assert_eq!(
            spec(&bvh, "splitmethod").choices,
            ["sah", "hlbvh", "middle", "equal"]
        );
        assert_eq!(
            default(&bvh, "maxnodeprims"),
            &Some(DefaultValue::Integers(&[8]))
        );
        // r4 cpu/aggregates/kdtree/accel.rs:521-527.
        let kdtree = rows("Accelerator", "kdtree").expect("kdtree");
        assert_eq!(
            names("Accelerator", "kdtree"),
            [
                "intersectcost",
                "traversalcost",
                "emptybonus",
                "maxprims",
                "maxdepth"
            ]
        );
        assert_eq!(
            default(&kdtree, "intersectcost"),
            &Some(DefaultValue::Integers(&[80]))
        );
        assert_eq!(
            default(&kdtree, "traversalcost"),
            &Some(DefaultValue::Integers(&[1]))
        );
        assert_eq!(
            default(&kdtree, "emptybonus"),
            &Some(DefaultValue::Floats(&[0.5]))
        );
        assert_eq!(
            default(&kdtree, "maxprims"),
            &Some(DefaultValue::Integers(&[1]))
        );
        assert_eq!(
            default(&kdtree, "maxdepth"),
            &Some(DefaultValue::Integers(&[-1]))
        );
    }

    fn sort<'a>(names: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
        let mut names: Vec<&str> = names.collect();
        names.sort_unstable();
        names
    }
}
