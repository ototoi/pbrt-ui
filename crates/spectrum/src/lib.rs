//! Spectrum-ish values to the linear RGB the previews and colour swatches
//! draw with (,; conductor F0 and
//! chromaticity in): rgb, blackbody, named builtin spectra and
//! λ/value pairs, each for one `Role`; a metal's normal-incidence F0
//! from its eta/k curves (`conductor_f0`); and the magnitude-free
//! colour a curve is (`chromaticity`).
//!
//! One implementation for Lit and the UI, so the viewport and a colour
//! box never disagree: `blackbody_rgb` is the function Lit's lights
//! carried, the named table is pbrt-r4's (16b0aab) spectra sampled at the
//! display channel wavelengths, and pairs read the way the role needs —
//! illuminants at the channel wavelengths like Lit, albedo and unbounded
//! values through a CIE 1931 integration normalized by the flat-spectrum
//! response (a flat curve of value `v` is exactly `[v, v, v]`).
//! `conductor_f0` integrates the per-wavelength Fresnel reflectance
//! against D65 and CIE 1931 the way scene-view's generated
//! `named_metals.rs` tables were, so the two agree within the table's
//! 1 nm verses 10 nm integration grids.
//!
//! No pbrt-r4 and no GPUI: pure `f64` arithmetic only.

mod named_curves;

/// The wavelengths, in nm, each display channel reads a spectrum at.
pub const CHANNEL_WAVELENGTHS: [f64; 3] = [630., 532., 465.];

/// What a spectrum is used for — the approximation follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A reflectance: shown as the colour the surface would have.
    Albedo,
    /// An unbounded value such as `eta`/`k`: colourified, never clamped.
    Unbounded,
    /// A light's radiance: exactly what Lit shades with, including level.
    Illuminant,
    /// The colour a curve is, independent of its magnitude: the CIE
    /// colour with negatives clamped to 0 and the biggest channel at 1.
    Chromaticity,
    /// A metal's normal-incidence reflectance F0 from its eta/k pair —
    /// the `conductor_f0` read `project.spectrum_color` serves.
    Conductor,
}

impl Role {
    /// The wire spelling of the role (`spectrum_color`'s `role` field).
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Albedo => "albedo",
            Self::Unbounded => "unbounded",
            Self::Illuminant => "illuminant",
            Self::Chromaticity => "chromaticity",
            Self::Conductor => "conductor",
        }
    }

    /// The role a wire `role` string names, if any.
    pub fn from_wire_str(text: &str) -> Option<Self> {
        match text {
            "albedo" => Some(Self::Albedo),
            "unbounded" => Some(Self::Unbounded),
            "illuminant" => Some(Self::Illuminant),
            "chromaticity" => Some(Self::Chromaticity),
            "conductor" => Some(Self::Conductor),
            _ => None,
        }
    }
}

/// One spectrum-ish value, whichever shape a parameter carries it in:
/// the four read paths (F0, reflectance, chromaticity, the role swatch)
/// all interpret the same value. `Named` is a builtin spectrum's r4
/// name (`metal-Au-k`, `glass-BK7`, …); `Pairs` is interleaved λ/value
/// samples; `Constant` is a bare scalar spectrum; `Rgb` is a display
/// colour given per channel.
#[derive(Clone, Copy, Debug)]
pub enum Value<'a> {
    Rgb([f64; 3]),
    Constant(f64),
    Named(&'a str),
    Pairs(&'a [f64]),
}

/// One named metal curve's interleaved λ/value samples, if the
/// generated table knows the name (r4's `metal-*-{eta,k}`, read by
/// `conductor_f0` wavelength by wavelength).
pub fn named_curve(name: &str) -> Option<&'static [f64]> {
    named_curves::curve(name)
}

/// A blackbody at `kelvin` as the linear RGB a swatch shows: Planck
/// radiance integrated against CIE 1931, normalized to luminance 1 — the
/// function used to shade lights in the Lit preview.
pub fn blackbody_rgb(kelvin: f64) -> [f64; 3] {
    // CIE 1931 2° color matching functions sampled from CIE 015:2018,
    // 380–780 nm at 10 nm intervals. Integrate Planck radiance, normalize
    // luminance Y, then convert XYZ to linear sRGB using the D65 matrix.
    let temperature = kelvin.clamp(1000., 40000.);
    let mut xyz = [0.; 3];
    for (index, matching) in CIE_1931.iter().enumerate() {
        let wavelength = (380. + index as f64 * 10.) * 1e-9;
        let relative =
            1. / (wavelength.powi(5) * (1.438776877e-2 / (wavelength * temperature)).exp_m1());
        for channel in 0..3 {
            xyz[channel] += matching[channel] * relative;
        }
    }
    if xyz[1] <= 0. || !xyz[1].is_finite() {
        return [1.; 3];
    }
    let xyz = xyz.map(|v| v / xyz[1]);
    srgb_d65_of(xyz)
}

/// CIE 1931 2° color matching functions, `[x̄, ȳ, z̄]` at 380, 390, …
/// 780 nm — the table blackbody_rgb has always integrated against.
#[rustfmt::skip]
const CIE_1931: [[f64; 3]; 41] = [
    [0.001368, 3.9e-5, 0.00645],
    [0.004243, 0.00012, 0.02005001],
    [0.01431, 0.000396, 0.06785001],
    [0.04351, 0.00121, 0.20739999],
    [0.13438, 0.004, 0.64560002],
    [0.28389999, 0.0116, 1.3856],
    [0.34828001, 0.023, 1.7470599],
    [0.3362, 0.038, 1.77211],
    [0.29080001, 0.06, 1.6691999],
    [0.19536, 0.09098, 1.28764],
    [0.09564, 0.13902, 0.81295007],
    [0.03201, 0.20802, 0.46518001],
    [0.0049, 0.32300001, 0.27200001],
    [0.0093, 0.50300002, 0.1582],
    [0.06327, 0.70999998, 0.07824999],
    [0.1655, 0.86199999, 0.04216],
    [0.2904, 0.954, 0.0203],
    [0.43344989, 0.99495012, 0.00875],
    [0.59450001, 0.995, 0.0039],
    [0.76209998, 0.95200002, 0.0021],
    [0.9163, 0.87, 0.00165],
    [1.0263, 0.75700003, 0.0011],
    [1.0621999, 0.63099998, 0.0008],
    [1.0026, 0.50300002, 0.00034],
    [0.85444993, 0.38100001, 0.00019],
    [0.64240003, 0.26499999, 5e-5],
    [0.4479, 0.175, 2e-5],
    [0.28349999, 0.107, 0.],
    [0.1649, 0.061, 0.],
    [0.0874, 0.032, 0.],
    [0.04677, 0.017, 0.],
    [0.0227, 0.00821, 0.],
    [0.01135916, 0.004102, 0.],
    [0.00579035, 0.002091, 0.],
    [0.00289933, 0.001047, 0.],
    [0.00143997, 0.00052, 0.],
    [0.00069008, 0.0002492, 0.],
    [0.0003323, 0.00012, 0.],
    [0.00016615, 0.00006, 0.],
    [0.00008308, 0.00003, 0.],
    [0.00004151, 0.00001499, 0.],
];

/// The linear-sRGB matrix blackbody_rgb has always converted with.
const fn srgb_d65_of(xyz: [f64; 3]) -> [f64; 3] {
    [
        3.2406 * xyz[0] - 1.5372 * xyz[1] - 0.4986 * xyz[2],
        -0.9689 * xyz[0] + 1.8758 * xyz[1] + 0.0415 * xyz[2],
        0.0557 * xyz[0] - 0.2040 * xyz[1] + 1.0570 * xyz[2],
    ]
}

/// A builtin named spectrum (r4 16b0aab names), if the table knows it:
/// the r4 value read at the display channel wavelengths. The samples are
/// held as `f32` exactly as scene-parse recorded them (printed once by a
/// throwaway program linked against r4; never a pbrt-ui dependency).
#[rustfmt::skip]
const NAMED: [(&str, [f32; 3]); 21] = [
    ("metal-Ag-eta", [0.1342497, 0.12965448, 0.13952407]),
    ("metal-Ag-k", [3.962828, 3.1942027, 2.6135478]),
    ("metal-Al-eta", [1.3570681, 0.8842938, 0.66048265]),
    ("metal-Al-k", [7.5877953, 6.46996, 5.6536283]),
    ("metal-Au-eta", [0.18369605, 0.45725846, 1.3723686]),
    ("metal-Au-k", [3.066341, 2.4083955, 1.8249992]),
    ("metal-Cu-eta", [0.24588375, 1.072285, 1.156862]),
    ("metal-Cu-k", [3.3783503, 2.5907342, 2.4572217]),
    ("metal-CuZn-eta", [0.445, 0.5682, 0.947]),
    ("metal-CuZn-k", [3.522, 2.588, 1.9200001]),
    ("metal-MgO-eta", [1.7347825, 1.7421676, 1.7501664]),
    ("metal-MgO-k", [0.0, 0.0, 0.0]),
    ("metal-TiO2-eta", [2.8749907, 2.9685428, 3.1175601]),
    ("metal-TiO2-k", [0.0, 0.0, 0.0]),
    ("glass-BAF10", [1.6672184, 1.6747586, 1.6830205]),
    ("glass-BK7", [1.5151856, 1.5194908, 1.5240455]),
    ("glass-F10", [1.7234025, 1.736679, 1.7519515]),
    ("glass-F11", [1.7788831, 1.7948356, 1.8134897]),
    ("glass-F5", [1.6686742, 1.6796174, 1.692149]),
    ("glass-FK51A", [1.4854028, 1.4884733, 1.4916879]),
    ("glass-LASF9", [1.8451338, 1.8590149, 1.8748455]),
];

/// A builtin named spectrum (metal eta / k, glass eta), if known.
pub fn named(name: &str) -> Option<[f64; 3]> {
    NAMED
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, rgb)| rgb.map(f64::from))
}

/// A builtin name the conductor paths cannot read at all: no λ/value
/// curve in the metal table and no channel triple in the named table —
/// an illuminant name such as `stdillum-D65` is not a refractive index
/// (nor is an unknown name). The Metal colour row answers the neutral
/// grey with a source saying so; Lit and the material thumbnail shade
/// the same grey, keeping the three surfaces in agreement.
pub fn named_is_not_a_refractive_index(name: &str) -> bool {
    named_curve(name).is_none() && named(name).is_none()
}

/// The λ/value samples of `pairs`, or nothing for a malformed list
/// (fewer than one pair, or an odd count).
fn points_of(pairs: &[f64]) -> Option<Vec<(f64, f64)>> {
    if pairs.len() < 2 || !pairs.len().is_multiple_of(2) {
        return None;
    }
    Some(
        pairs
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| (chunk[0], chunk[1]))
            .collect(),
    )
}

/// The curve read at `lambda`, piecewise linearly, held constant past
/// either end, as pbrt's `SampledSpectrum` does.
fn curve_at(points: &[(f64, f64)], lambda: f64) -> f64 {
    let (first, last) = (points[0], points[points.len() - 1]);
    if lambda <= first.0 {
        return first.1;
    }
    if lambda >= last.0 {
        return last.1;
    }
    let index = points
        .windows(2)
        .position(|window| lambda <= window[1].0)
        .unwrap_or(0);
    let ((l0, v0), (l1, v1)) = (points[index], points[index + 1]);
    let t = if l1 > l0 {
        (lambda - l0) / (l1 - l0)
    } else {
        0.
    };
    v0 + t * (v1 - v0)
}

/// λ/value pairs read at the display channel wavelengths, as pbrt's
/// `SampledSpectrum` previews do — the role Lit's lights resolve with,
/// leaving the level as written.
pub fn channel_rgb(pairs: &[f64]) -> Option<[f64; 3]> {
    let points = points_of(pairs)?;
    Some(CHANNEL_WAVELENGTHS.map(|lambda| curve_at(&points, lambda)))
}

/// λ/value pairs as the linear RGB a `role` swatch shows. Illuminants
/// keep Lit's channel read; albedo and unbounded values integrate
/// against CIE 1931 and normalize by the flat-spectrum response, and
/// albedo additionally clamps out-of-gamut negatives to 0. The
/// conductor role reads its curves through `conductor_f0` in core, not
/// here. Malformed pairs (fewer than one pair, or an odd count) resolve
/// to nothing.
pub fn pairs_rgb(pairs: &[f64], role: Role) -> Option<[f64; 3]> {
    match role {
        Role::Illuminant => channel_rgb(pairs),
        Role::Albedo => cie_rgb(pairs).map(|rgb| rgb.map(|v| v.max(0.))),
        Role::Unbounded | Role::Conductor => cie_rgb(pairs),
        Role::Chromaticity => chromaticity(Value::Pairs(pairs)),
    }
}

/// The CIE 1931 integration albedo and unbounded pairs show: the curve
/// sampled at the CIE table's wavelengths, converted by the D65
/// linear-sRGB matrix, then divided per channel by the flat spectrum's
/// conversion so a flat curve of value `v` reads as exactly `[v, v, v]`
/// — the level survives, the chromaticity carries the colour.
fn cie_rgb(pairs: &[f64]) -> Option<[f64; 3]> {
    let points = points_of(pairs)?;
    let spectrum = |lambda: f64| curve_at(&points, lambda);
    let mut xyz = [0.; 3];
    let mut flat_xyz = [0.; 3];
    for (index, matching) in CIE_1931.iter().enumerate() {
        let value = spectrum(380. + index as f64 * 10.);
        for channel in 0..3 {
            xyz[channel] += matching[channel] * value;
            flat_xyz[channel] += matching[channel];
        }
    }
    let rgb = srgb_d65_of(xyz);
    let flat = srgb_d65_of(flat_xyz);
    let mut out = [0.; 3];
    for channel in 0..3 {
        out[channel] = if flat[channel] > 0. {
            rgb[channel] / flat[channel]
        } else {
            0.
        };
    }
    Some(out)
}

/// The λ/value points of any value, ascending by wavelength, with the
/// out-of-domain rule the value's own source reads with: pairs and
/// named curves are zero outside their domain — r4's
/// `PiecewiseLinearSpectrum` — while rgb channel samples and constants
/// join into a curve held constant past its ends, closer to r4's
/// unbounded spectrum rgb builds (the approximation picked closest to
/// the named-metal tables; held ends land within ~0.04, zero ends push
/// blue out by ~0.5). Nothing drawable — a malformed pair list or an
/// unknown name — resolves to nothing.
fn curve_points(value: Value) -> Option<(Vec<(f64, f64)>, bool)> {
    match value {
        Value::Rgb([r, g, b]) => Some((vec![(465., b), (532., g), (630., r)], false)),
        Value::Constant(value) => Some((vec![(380., value), (780., value)], false)),
        Value::Named(name) => Some((points_of(named_curve(name)?)?, true)),
        Value::Pairs(pairs) => Some((points_of(pairs)?, true)),
    }
}

/// The curve read at `lambda`, piecewise linearly, with samples outside
/// the curve's domain reading 0 — r4's `PiecewiseLinearSpectrum` rule.
/// eta/k values live in curves whose ends are not the visible band's,
/// and holding the ends on a conductor would paint a mirror of nothing.
fn sample_zero_outside(points: &[(f64, f64)], lambda: f64) -> f64 {
    let (first, last) = (points[0], points[points.len() - 1]);
    if lambda < first.0 || lambda > last.0 {
        return 0.;
    }
    if points.len() == 1 {
        return first.1;
    }
    let index = points
        .windows(2)
        .position(|window| lambda <= window[1].0)
        .unwrap_or(0);
    let ((l0, v0), (l1, v1)) = (points[index], points[index + 1]);
    let t = if l1 > l0 {
        (lambda - l0) / (l1 - l0)
    } else {
        0.
    };
    v0 + t * (v1 - v0)
}

/// A metal's normal-incidence reflectance F0 as the linear RGB a
/// swatch shows: per-wavelength Fresnel of the eta/medium and k/medium
/// curves, weighted by D65 × CIE 1931 at the table's 10 nm wavelengths
/// and normalized by the D65-weighted luminance ∫ D65·ȳ — the same
/// integration scene-view's generated `named_metals.rs` tables carry
/// (within 2e-3), so Lit and the Metal colour row agree. rgb values
/// read as their three display-channel samples joined into a curve
/// (held past its ends, as above); a glass name (the named table's
/// non-metal channel triples) reads the same way — the rgb path's own
/// reading; a malformed pair list, an unknown name, or
/// a non-positive medium resolves to nothing.
pub fn conductor_f0(eta: Value, k: Value, medium: f64) -> Option<[f64; 3]> {
    if !medium.is_finite() || medium <= 0. {
        return None;
    }
    // A side with no λ/value curve but channel samples (the glass
    // names) reads them the way an rgb-declared side does.
    let side = |value: Value| match value {
        Value::Named(name) if named_curve(name).is_none() => curve_points(Value::Rgb(named(name)?)),
        other => curve_points(other),
    };
    let eta = side(eta)?;
    let k = side(k)?;
    Some(conductor_f0_of(|lambda| {
        f0_at(
            sample_curve(&eta, lambda) / medium,
            sample_curve(&k, lambda) / medium,
        )
    }))
}

/// A conductor declared by `reflectance` shows that reflectance's own
/// F0. In vacuum r4's form — eta 1, k = 2√r/√(1−r), r clamped to
/// [0, 0.9999] — collapses back to r, so the answer is the value's
/// albedo colour clamped, exactly what the reflectance row's colour box
/// shows. An rgb or constant reflectance under a coat evaluates the
/// divided construction per display channel — the way r4 reads its own
/// channel values, which a mixed D65/CIE integral would drift 9% off
/// in blue; a named spectrum or λ/value pairs keep the wavelength
/// integral, where the mixed reading is the point.
pub fn reflectance_f0(reflectance: Value, medium: f64) -> Option<[f64; 3]> {
    if !medium.is_finite() || medium <= 0. {
        return None;
    }
    if medium == 1. {
        return Some(
            match reflectance {
                Value::Rgb(rgb) => rgb,
                Value::Constant(value) => [value; 3],
                Value::Named(name) => named(name)?,
                Value::Pairs(pairs) => pairs_rgb(pairs, Role::Albedo)?,
            }
            .map(|v| v.clamp(0., 0.9999)),
        );
    }
    match reflectance {
        Value::Rgb(rgb) => return Some(rgb.map(|v| reflectance_channel(v, medium))),
        Value::Constant(value) => return Some([reflectance_channel(value, medium); 3]),
        Value::Named(_) | Value::Pairs(_) => {}
    }
    let reflectance = curve_points(reflectance)?;
    Some(conductor_f0_of(|lambda| {
        reflectance_channel(sample_curve(&reflectance, lambda), medium)
    }))
}

/// One wavelength's (or one channel's) reflectance as the F0 of r4's
/// divided construction: reflectance r becomes eta 1, k = 2√r/√(1−r),
/// both divided by the interface index. (1−r) after the clamp stays
/// ≥ 1e-4, and an r clamped out to 0 reads k 0, as r4's clamp does.
fn reflectance_channel(r: f64, medium: f64) -> f64 {
    let r = r.clamp(0., 0.9999);
    let k = if r > 0. {
        2. * r.sqrt() / (1. - r).sqrt() / medium
    } else {
        0.
    };
    f0_at(1. / medium, k)
}

/// The Fresnel reflectance at normal incidence: the eta/k form shared
/// by both conductor declarations.
fn f0_at(eta: f64, k: f64) -> f64 {
    ((eta - 1.).powi(2) + k * k) / ((eta + 1.).powi(2) + k * k)
}

/// One read of the D65-weighted F0 curve `sampled` returns per
/// wavelength, integrated into RGB the named_metal tables' way.
fn conductor_f0_of<F: Fn(f64) -> f64>(sampled: F) -> [f64; 3] {
    let mut xyz = [0.; 3];
    let mut norm = 0.;
    for (index, matching) in CIE_1931.iter().enumerate() {
        let lambda = 380. + index as f64 * 10.;
        let f0 = sampled(lambda);
        let weight = f0 * named_curves::D65[index];
        for channel in 0..3 {
            xyz[channel] += weight * matching[channel];
        }
        norm += named_curves::D65[index] * matching[1];
    }
    let normalized = xyz.map(|v| v / norm.max(f64::MIN_POSITIVE));
    srgb_d65_of(normalized)
}

/// One curve, read the way its value's own source does: pairs and named
/// curves read piecewise-linearly, 0 outside their domain (r4's
/// `PiecewiseLinearSpectrum`), rgb channel samples and constants hold
/// their ends (see [`curve_points`]).
fn sample_curve(curve: &(Vec<(f64, f64)>, bool), lambda: f64) -> f64 {
    if curve.1 {
        sample_zero_outside(&curve.0, lambda)
    } else {
        curve_at(&curve.0, lambda)
    }
}

/// One value's sample at `lambda`: the read a scalar takes from a
/// spectral declaration — pbrt's coatedconductor reads its interface's
/// index at one wavelength. An unreadable value (unknown name, malformed
/// pairs) resolves to nothing.
pub fn curve_point(value: Value, lambda: f64) -> Option<f64> {
    if !lambda.is_finite() {
        return None;
    }
    match value {
        // An rgb triple (and the glass names' channel triples) read
        // their display-channel sample nearest the wavelength.
        Value::Rgb(rgb) => Some(rgb[nearest_channel(lambda)]),
        Value::Constant(value) => Some(value),
        Value::Named(name) => match named_curve(name) {
            Some(curve) => curve_point(Value::Pairs(curve), lambda),
            None => named(name).map(|rgb| rgb[nearest_channel(lambda)]),
        },
        Value::Pairs(pairs) => Some(sample_curve(&(points_of(pairs)?, true), lambda)),
    }
}

/// The display channel nearest `lambda`: indexes `CHANNEL_WAVELENGTHS`
/// from a finite λ.
fn nearest_channel(lambda: f64) -> usize {
    CHANNEL_WAVELENGTHS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (**a - lambda).abs().total_cmp(&(**b - lambda).abs()))
        .map(|(index, _)| index)
        .unwrap_or(1)
}

/// The colour a value is, without its magnitude: the value's RGB — the
/// CIE 1931 integration for curves, the value itself for rgb/constant
/// — with negatives clamped to 0 and then scaled so the biggest channel
/// is 1. A flat spectrum is white, a zero spectrum black, and a curve of
/// values beyond 1 (a metal's k) keeps its hue instead of saturating.
pub fn chromaticity(value: Value) -> Option<[f64; 3]> {
    Some(chromaticity_of_rgb(match value {
        Value::Rgb(rgb) => rgb,
        Value::Constant(value) => [value; 3],
        // A named curve integrates its own points; the r4-glass channel
        // triples sit at the display wavelengths only.
        Value::Named(name) => match named_curve(name) {
            Some(curve) => cie_rgb(curve)?,
            None => named(name)?,
        },
        Value::Pairs(pairs) => cie_rgb(pairs)?,
    }))
}

/// An RGB's colour, its magnitude dropped: negatives clamped to 0, then
/// scaled so the biggest channel is 1. An all-clamped-out (or zero)
/// colour stays black: it has no colour to keep.
pub fn chromaticity_of_rgb(rgb: [f64; 3]) -> [f64; 3] {
    let rgb = rgb.map(|v| v.max(0.));
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    if max <= 0. || !max.is_finite() {
        return [0.; 3];
    }
    rgb.map(|v| v / max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blackbody_is_luminance_normalized() {
        let rgb = blackbody_rgb(6500.);
        // Luminance is Y: recompute XYZ from the matrix's middle column and
        // require the normalization blackbody_rgb documents (Y == 1).
        // XYZ->RGB matrix rows give three equations; solve for XYZ with the
        // inverse of the 3x3 matrix blackbody_rgb applies.
        let m = [
            [3.2406, -1.5372, -0.4986],
            [-0.9689, 1.8758, 0.0415],
            [0.0557, -0.2040, 1.0570],
        ];
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        let xyz = [
            (rgb[0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                + rgb[1] * (m[0][2] * m[2][1] - m[0][1] * m[2][2])
                + rgb[2] * (m[0][1] * m[1][2] - m[0][2] * m[1][1]))
                / det,
            (rgb[0] * (m[1][2] * m[2][0] - m[1][0] * m[2][2])
                + rgb[1] * (m[0][0] * m[2][2] - m[0][2] * m[2][0])
                + rgb[2] * (m[0][2] * m[1][0] - m[0][0] * m[1][2]))
                / det,
            (rgb[0] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
                + rgb[1] * (m[0][1] * m[2][0] - m[0][0] * m[2][1])
                + rgb[2] * (m[0][0] * m[1][1] - m[0][1] * m[1][0]))
                / det,
        ];
        assert!((xyz[1] - 1.).abs() < 1e-9, "Y is {xyz:?}");
    }

    #[test]
    fn blackbody_is_warm_below_daylight_and_cool_above() {
        let warm = blackbody_rgb(2700.);
        let daylight = blackbody_rgb(6500.);
        let cool = blackbody_rgb(12000.);
        assert!(warm[0] > warm[2], "2700 K must lean red: {warm:?}");
        assert!(cool[2] > warm[2], "12000 K must beat 2700 K in blue");
        assert!(
            (daylight[0] - 1.).abs() < 0.1 && (daylight[2] - 1.).abs() < 0.1,
            "6500 K is near white: {daylight:?}"
        );
    }

    #[test]
    fn named_spectra_read_the_r4_table() {
        assert_eq!(
            named("metal-Cu-eta"),
            Some([0.24588375f32 as f64, 1.072285f32 as f64, 1.156862f32 as f64,])
        );
        assert_eq!(named("glass-BK7").map(|v| v[1]), Some(1.5194908f32 as f64));
        assert_eq!(named("metal-Cu"), None, "exact names only, as r4");
    }

    #[test]
    fn illuminant_pairs_read_the_channel_wavelengths() {
        // Lit's area-light pins: 630 nm and 532 nm held at 0.5, 465 nm in
        // the ramp at 0.25 + 0.65·0.25.
        let rgb = pairs_rgb(&[400., 0.25, 500., 0.5], Role::Illuminant).unwrap();
        for (got, want) in rgb.into_iter().zip([0.5, 0.5, 0.4125]) {
            assert!((got - want).abs() < 1e-4, "{rgb:?}");
        }
        // A flat spectrum is exactly its level, the lit-units GPU contract.
        assert_eq!(
            pairs_rgb(&[400., 0.5, 700., 0.5], Role::Illuminant),
            Some([0.5; 3])
        );
        // One pair holds at every channel, as the old `sampled` did.
        assert_eq!(pairs_rgb(&[400., 1.], Role::Illuminant), Some([1.; 3]));
    }

    #[test]
    fn pairs_reject_malformed_input() {
        for pairs in [&[][..], &[400.][..], &[1., 2., 3.][..]] {
            assert!(pairs_rgb(pairs, Role::Illuminant).is_none());
            assert!(pairs_rgb(pairs, Role::Albedo).is_none());
            assert!(pairs_rgb(pairs, Role::Unbounded).is_none());
        }
    }

    #[test]
    fn albedo_pairs_integrate_a_flat_spectrum_exactly() {
        let rgb = pairs_rgb(&[380., 2., 780., 2.], Role::Albedo).unwrap();
        assert_eq!(rgb, [2.; 3]);
        let rgb = pairs_rgb(&[400., 1.5, 700., 1.5], Role::Albedo).unwrap();
        for channel in rgb {
            assert!((channel - 1.5).abs() < 1e-9, "{rgb:?}");
        }
    }

    #[test]
    fn albedo_pairs_keep_their_chromaticity() {
        // Rising toward the red end, the curve reads red-dominant.
        let red = pairs_rgb(&[400., 0., 600., 1., 700., 1.], Role::Albedo);
        let red = red.unwrap();
        assert!(red[0] > red[2] && red[0] > red[1], "{red:?}");
        // Concentrated in the blue ramp, it reads blue-dominant.
        let blue = pairs_rgb(&[400., 1., 500., 1., 600., 0., 700., 0.], Role::Albedo);
        let blue = blue.unwrap();
        assert!(blue[2] > blue[0] && blue[2] > blue[1], "{blue:?}");
    }

    #[test]
    fn albedo_pairs_clamp_out_of_gamut_negatives() {
        // A 550 nm spike is a spectral green outside sRGB: blue goes
        // negative before the clamp.
        let spike = pairs_rgb(&[540., 0., 550., 1., 560., 0.], Role::Albedo);
        let spike = spike.unwrap();
        assert!(spike.iter().all(|v| *v >= 0.), "{spike:?}");
        assert!(spike[1] > spike[0] && spike[1] > 0., "{spike:?}");
        let raw = pairs_rgb(&[540., 0., 550., 1., 560., 0.], Role::Unbounded);
        let raw = raw.unwrap();
        assert!(raw[2] < 0., "the unclamped blue channel is {raw:?}");
    }

    #[test]
    fn unbounded_pairs_keep_the_absolute_values() {
        let rgb = pairs_rgb(&[300., 1.5, 826., 1.5], Role::Unbounded);
        let rgb = rgb.unwrap();
        for channel in rgb {
            assert!((channel - 1.5).abs() < 1e-9, "{rgb:?}");
        }
    }

    #[test]
    fn roles_round_trip_the_wire_strings() {
        for role in [
            Role::Albedo,
            Role::Unbounded,
            Role::Illuminant,
            Role::Chromaticity,
            Role::Conductor,
        ] {
            assert_eq!(Role::from_wire_str(role.as_wire_str()), Some(role));
        }
        assert_eq!(Role::from_wire_str("bogus"), None);
    }

    /// scene-view's `named_metals.rs` tables — generated by
    /// tools/named-metal-f0.py from the same r4 curves this crate now
    /// holds — as the reference `conductor_f0` must reproduce. 2e-3 is
    /// tolerance for the D65/CIE 10 nm integration against
    /// the tables' 1 nm one.
    #[test]
    fn conductor_f0_reproduces_the_named_metal_tables() {
        #[rustfmt::skip]
        const VACUUM: &[(&str, [f64; 3])] = &[
            ("metal-Cu-eta", [0.955325285, 0.637578994, 0.538202018]),
            ("metal-Au-eta", [1.02188775, 0.782443963, 0.34342196]),
            ("metal-Ag-eta", [0.971519259, 0.959932253, 0.915376763]),
            ("metal-Al-eta", [0.913181406, 0.92149393, 0.924523882]),
        ];
        #[rustfmt::skip]
        const COAT_1_5: &[(&str, [f64; 3])] = &[
            ("metal-Cu-eta", [0.934587532, 0.549326685, 0.438487879]),
            ("metal-Au-eta", [1.0098443, 0.734506672, 0.234874706]),
            ("metal-Ag-eta", [0.960356791, 0.945695126, 0.893870603]),
            ("metal-Al-eta", [0.874831502, 0.887414021, 0.893017231]),
        ];
        for (medium, table) in [(1., VACUUM), (1.5, COAT_1_5)] {
            for (eta, want) in table {
                // `metal` itself contains `eta`, so build the k name from
                // the eta name's suffix, not a substring replace.
                let k = format!("{}k", eta.strip_suffix("eta").unwrap());
                let got = conductor_f0(Value::Named(eta), Value::Named(&k), medium)
                    .unwrap_or_else(|| panic!("no F0 for {eta}"));
                for (got, want) in got.into_iter().zip(want) {
                    assert!(
                        (got - want).abs() < 2e-3,
                        "{eta} in {medium}: got {got}, table {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn conductor_f0_reads_lambda_value_pairs_like_the_named_curves() {
        // The spd files' pairs are the same data as the named curves
        // (r4's Au_eta and killeroos' Au.eta.spd agree), so a pairs F0 is
        // equal to the named one.
        let eta = named_curve("metal-Au-eta").unwrap();
        let k = named_curve("metal-Au-k").unwrap();
        let from_pairs = conductor_f0(Value::Pairs(eta), Value::Pairs(k), 1.).unwrap();
        let named =
            conductor_f0(Value::Named("metal-Au-eta"), Value::Named("metal-Au-k"), 1.).unwrap();
        assert_eq!(from_pairs, named);
        // A missing or malformed curve resolves to nothing.
        assert_eq!(
            conductor_f0(
                Value::Named("metal-nope-eta"),
                Value::Named("metal-Cu-k"),
                1.
            ),
            None
        );
        assert_eq!(
            conductor_f0(
                Value::Pairs(&[]),
                Value::Pairs(&[400., 1., 500., 2., 3.]),
                1.
            ),
            None
        );
    }

    #[test]
    fn conductor_f0_reads_glass_names_as_channel_samples_like_the_rgb_path() {
        // The glass names hold channel triples, not λ/value curves: a
        // glass eta (the dielectric default names) reads as its three
        // per-channel samples exactly the way an rgb-declared eta does,
        // so a glass with a metal's k resolves instead of failing.
        let glass = named("glass-BK7").unwrap();
        let from_name =
            conductor_f0(Value::Named("glass-BK7"), Value::Named("metal-Cu-k"), 1.).unwrap();
        let from_rgb = conductor_f0(Value::Rgb(glass), Value::Named("metal-Cu-k"), 1.).unwrap();
        assert_eq!(from_name, from_rgb);
        // The glass eta counts: its reading is not r4's copper fill.
        let copper =
            conductor_f0(Value::Named("metal-Cu-eta"), Value::Named("metal-Cu-k"), 1.).unwrap();
        assert_ne!(from_name, copper);
        // A metal name keeps reading its own λ/value curve — more data
        // than the channel triple, so the two must differ.
        let from_curve =
            conductor_f0(Value::Named("metal-Au-eta"), Value::Named("metal-Au-k"), 1.).unwrap();
        let from_channels = conductor_f0(
            Value::Rgb(named("metal-Au-eta").unwrap()),
            Value::Rgb(named("metal-Au-k").unwrap()),
            1.,
        )
        .unwrap();
        assert_ne!(from_curve, from_channels);
    }

    /// a name with neither a curve nor channel samples is
    /// not a refractive index — the three conductor surfaces (Metal
    /// colour row, thumbnail, Lit) answer the neutral grey with a
    /// source saying so, instead of an error or a copper fallback.
    #[test]
    fn names_without_a_curve_or_channels_are_not_a_refractive_index() {
        for name in ["metal-Cu-eta", "metal-Ag-k", "glass-BK7", "glass-LASF9"] {
            assert!(!named_is_not_a_refractive_index(name), "{name} reads");
        }
        for name in [
            "stdillum-D65",
            "illum-D65",
            "cie1931-bar-y",
            "metal-nope-eta",
        ] {
            assert!(
                named_is_not_a_refractive_index(name),
                "{name} reads nothing"
            );
        }
        // Such a name still resolves to no F0, whatever the other side.
        assert_eq!(
            conductor_f0(Value::Named("stdillum-D65"), Value::Named("metal-Cu-k"), 1.),
            None
        );
        assert_eq!(
            conductor_f0(Value::Named("glass-BK7"), Value::Named("stdillum-D65"), 1.),
            None
        );
    }

    #[test]
    fn conductor_f0_mixes_each_given_curve() {
        // eta of one metal and k of another blend rather than one
        // masquerading as the whole metal.
        let pure_au =
            conductor_f0(Value::Named("metal-Au-eta"), Value::Named("metal-Au-k"), 1.).unwrap();
        let pure_cu =
            conductor_f0(Value::Named("metal-Cu-eta"), Value::Named("metal-Cu-k"), 1.).unwrap();
        let mixed =
            conductor_f0(Value::Named("metal-Au-eta"), Value::Named("metal-Cu-k"), 1.).unwrap();
        assert_ne!(mixed, pure_au);
        assert_ne!(mixed, pure_cu);
    }

    #[test]
    fn rgb_conductor_f0_approximates_the_table() {
        // The channel values of r4's Au table samples as an rgb triple:
        // within ~0.04 of the D65-integrated table row (
        // ~0.05 rgb allowance; r4 builds an unbounded spectrum from rgb,
        // which the three curve points approximate).
        let eta = named("metal-Au-eta").unwrap();
        let k = named("metal-Au-k").unwrap();
        let got = conductor_f0(Value::Rgb(eta), Value::Rgb(k), 1.).unwrap();
        let want = [1.02188775, 0.782443963, 0.34342196];
        for (got, want) in got.into_iter().zip(want) {
            assert!((got - want).abs() < 0.05, "got {got}, table {want}");
        }
    }

    #[test]
    fn reflectance_f0_is_the_albedo_colour_clamped_to_r4s_maximum() {
        assert_eq!(
            reflectance_f0(Value::Rgb([0.9, 0.6, 0.3]), 1.).unwrap(),
            [0.9, 0.6, 0.3]
        );
        assert_eq!(
            reflectance_f0(Value::Rgb([1.2, -0.1, 0.3]), 1.).unwrap(),
            [0.9999, 0., 0.3]
        );
        assert_eq!(reflectance_f0(Value::Constant(0.5), 1.).unwrap(), [0.5; 3]);
        let flat = reflectance_f0(Value::Pairs(&[380., 0.5, 780., 0.5]), 1.).unwrap();
        for channel in flat {
            assert!((channel - 0.5).abs() < 1e-9, "{flat:?}");
        }
        assert_eq!(
            reflectance_f0(Value::Named("glass-BK7"), 1.).unwrap(),
            [0.9999; 3]
        );
        assert_eq!(reflectance_f0(Value::Named("metal-nope"), 1.), None);
        assert_eq!(
            reflectance_f0(Value::Pairs(&[400.]), 1.),
            None,
            "an odd count is no pair list"
        );
    }

    #[test]
    fn reflectance_f0_under_a_coat_divides_like_r4() {
        let bare = reflectance_f0(Value::Rgb([0.5, 0.5, 0.5]), 1.).unwrap();
        assert_eq!(bare, [0.5; 3]);
        // r4 builds eta 1, k = 2√r/√(1−r) from a reflectance, then the
        // coat divides both by the interface index: r 0.5 under 1.5 is
        // eta 2/3, k 4/3, F0 = (1/9 + 16/9)/(25/9 + 16/9) = 17/41.
        let coated = reflectance_f0(Value::Rgb([0.5, 0.5, 0.5]), 1.5).unwrap();
        for channel in coated {
            assert!((channel - 17. / 41.).abs() < 1e-3, "{coated:?}");
        }
        assert!(coated[0] < bare[0], "the coat reflects less, {coated:?}");
        // A rising spectral reflectance keeps its curve's shape.
        let red = reflectance_f0(Value::Pairs(&[380., 0.2, 780., 0.8]), 1.).unwrap();
        assert!(red[0] > red[2] && red[0] > 0.25 && red[0] < 0.8, "{red:?}");
    }

    #[test]
    fn reflectance_f0_under_a_coat_reads_rgb_per_channel() {
        // An rgb-declared reflectance evaluates its channels — r4's own
        // display-channel construction — not the D65/CIE integral the
        // named-metal tables use: a rising reflectance's mixed spectrum
        // reads blue out by 9%, past the r4 reference scenes' band the
        // grey 17/41 case cannot show.
        let coated = reflectance_f0(Value::Rgb([0.8, 0.6, 0.3]), 1.5).unwrap();
        let channel = |r: f64| {
            let r = r.clamp(0., 0.9999);
            let k = if r > 0. {
                2. * r.sqrt() / (1. - r).sqrt() / 1.5
            } else {
                0.
            };
            f0_at(1. / 1.5, k)
        };
        assert!((coated[0] - channel(0.8)).abs() < 1e-12, "{coated:?}");
        assert!((coated[1] - channel(0.6)).abs() < 1e-12, "{coated:?}");
        assert!((coated[2] - channel(0.3)).abs() < 1e-12, "{coated:?}");
        // A constant stays the flat curve's single value.
        let constant = reflectance_f0(Value::Constant(0.5), 1.5).unwrap();
        for channel in constant {
            assert!((channel - 17. / 41.).abs() < 1e-9, "{constant:?}");
        }
    }

    #[test]
    fn curve_point_reads_one_wavelength_of_each_value_shape() {
        // λ/value pairs (and named curves) sample piecewise-linearly, 0
        // outside their domain, as r4's PiecewiseLinearSpectrum does.
        let ramp = [400., 0., 600., 1.];
        assert_eq!(curve_point(Value::Pairs(&ramp), 500.), Some(0.5));
        assert_eq!(curve_point(Value::Pairs(&ramp), 400.), Some(0.));
        assert_eq!(curve_point(Value::Pairs(&ramp), 600.), Some(1.));
        assert_eq!(curve_point(Value::Pairs(&ramp), 300.), Some(0.));
        assert_eq!(curve_point(Value::Pairs(&ramp), 700.), Some(0.));
        let curve = named_curve("metal-Au-eta").unwrap();
        assert_eq!(
            curve_point(Value::Named("metal-Au-eta"), 550.),
            curve_point(Value::Pairs(curve), 550.),
        );
        // An rgb triple reads its display-channel sample, a constant its
        // own value, a glass name its nearest channel; an unknown name
        // has nothing to read.
        assert_eq!(curve_point(Value::Rgb([0.1, 0.2, 0.3]), 550.), Some(0.2));
        assert_eq!(curve_point(Value::Rgb([0.1, 0.2, 0.3]), 630.), Some(0.1));
        assert_eq!(curve_point(Value::Rgb([0.1, 0.2, 0.3]), 465.), Some(0.3));
        assert_eq!(curve_point(Value::Constant(1.4), 415.), Some(1.4));
        assert_eq!(
            curve_point(Value::Named("glass-BK7"), 550.),
            Some(1.5194908f32 as f64)
        );
        assert_eq!(curve_point(Value::Named("metal-nope"), 550.), None);
        assert_eq!(
            curve_point(Value::Pairs(&[2., 2., 2.5]), 550.),
            None,
            "odd count"
        );
    }

    #[test]
    fn chromaticity_is_independent_of_magnitude() {
        let curve = &[298.75, 5.0, 350., 4.2, 550., 1.1, 770., 3.0, 855., 2.4];
        let scales = [0.5, 1., 10.];
        let colours = scales.map(|scale| {
            let scaled: Vec<f64> = curve
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|pair| [pair[0], pair[1] * scale])
                .collect();
            chromaticity(Value::Pairs(&scaled)).unwrap()
        });
        for colour in &colours[1..] {
            for (got, want) in colour.iter().zip(colours[0]) {
                assert!((got - want).abs() < 1e-12, "{colours:?}");
            }
        }
        // A flat spectrum is white whatever its level; zero is black.
        assert_eq!(
            chromaticity(Value::Pairs(&[380., 2., 780., 2.])).unwrap(),
            [1.; 3]
        );
        assert_eq!(
            chromaticity(Value::Pairs(&[380., 0., 780., 0.])).unwrap(),
            [0.; 3]
        );
        assert_eq!(chromaticity(Value::Constant(3.)).unwrap(), [1.; 3]);
        assert_eq!(chromaticity(Value::Constant(0.)).unwrap(), [0.; 3]);
        assert_eq!(
            chromaticity(Value::Rgb([-0.2, 0.4, 0.8])).unwrap(),
            [0., 0.5, 1.]
        );
        // Au.k's unbounded values do not saturate: the biggest channel
        // lands on exactly 1.
        let k = named_curve("metal-Au-k").unwrap();
        let colour = chromaticity(Value::Pairs(k)).unwrap();
        assert_eq!(colour[0], 1.);
        assert!(colour[0] > colour[1] && colour[1] > colour[2], "{colour:?}");
        assert_eq!(chromaticity(Value::Named("metal-nope-k")), None);
        assert_eq!(chromaticity(Value::Pairs(&[550.])), None);
    }
}
