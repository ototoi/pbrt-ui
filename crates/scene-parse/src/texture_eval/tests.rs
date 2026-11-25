use super::noise::{fbm, noise, turbulence};
use super::*;

/// r4's values at fixed points, recorded from a program linked against
/// pbrt-r4 211eb9e (4.5.2).
/// Columns: point, noise, fbm(ω 0.5, 8), turbulence(ω 0.5, 8),
/// fbm(ω 0.7, 3), all with zero footprints.
const R4_NOISE: [([f32; 3], f32, f32, f32, f32); 8] = [
    (
        [0.25, 0.5, 0.0],
        -0.103515625,
        -0.09772869,
        0.12787768,
        -0.110334925,
    ),
    (
        [1.3, 2.7, 0.4],
        0.57116413,
        0.8901683,
        0.8913029,
        0.95968014,
    ),
    (
        [-3.2, 0.75, 1.9],
        0.276148,
        0.17213123,
        0.47984517,
        0.20946771,
    ),
    (
        [10.5, -4.25, 7.125],
        0.48423952,
        0.30662462,
        0.8642567,
        0.13088767,
    ),
    ([0.0, 0.0, 0.0], 0.0, 0.0, 0.00078125, 0.0),
    (
        [0.5, 0.5, 0.5],
        -0.125,
        -0.11803757,
        0.18843202,
        -0.13751322,
    ),
    (
        [123.456, 78.9, -0.001],
        -0.21317613,
        -0.13563302,
        0.54481596,
        -0.15020354,
    ),
    ([2.5, 3.5, 4.5], -0.125, -0.2083144, 0.30276605, -0.30143225),
];

#[test]
fn noise_fbm_and_turbulence_match_r4() {
    let zero = [0.; 3];
    for (p, n, f, t, f3) in R4_NOISE {
        assert_eq!(noise(p[0], p[1], p[2]), n, "noise {p:?}");
        assert_eq!(fbm(p, zero, zero, 0.5, 8), f, "fbm {p:?}");
        assert_eq!(turbulence(p, zero, zero, 0.5, 8), t, "turbulence {p:?}");
        assert_eq!(fbm(p, zero, zero, 0.7, 3), f3, "fbm 0.7/3 {p:?}");
    }
}

fn at(u: f32, v: f32) -> Context {
    Context {
        p: [u, v, 0.],
        n: [0., 0., 1.],
        uv: [u, v],
    }
}

fn constant(value: f32) -> Operand {
    Operand::Constant([value; 3])
}

#[test]
fn uv_mapping_scales_and_offsets_and_checkerboard_alternates() {
    let mapping = Mapping2D::Uv {
        su: 4.,
        sv: 4.,
        du: 0.,
        dv: 0.,
    };
    assert_eq!(mapping.map(&at(0.3, 0.6)), [1.2, 2.4]);
    let checks = Texture::Checkerboard2D {
        mapping,
        tex1: constant(1.),
        tex2: constant(0.),
    };
    assert_eq!(checks.evaluate(&at(0.1, 0.1)), [1.; 3], "cell (0, 0)");
    assert_eq!(checks.evaluate(&at(0.3, 0.1)), [0.; 3], "cell (1, 0)");
    assert_eq!(checks.evaluate(&at(0.3, 0.3)), [1.; 3], "cell (1, 1)");
    // r4's i32 `%`: an odd negative sum is tex2 too.
    assert_eq!(checks.evaluate(&at(-0.1, 0.1)), [0.; 3]);
}

#[test]
fn planar_and_spherical_mappings_follow_the_declaration_transform() {
    // A texture declared under Translate 1 0 0: texture space is world - 1.
    let mut inverse = IDENTITY;
    inverse[0][3] = -1.;
    let planar = Mapping2D::Planar {
        texture_from_world: inverse,
        vs: [1., 0., 0.],
        vt: [0., 1., 0.],
        ds: 0.5,
        dt: 0.,
    };
    assert_eq!(planar.map(&at(3., 2.)), [2.5, 2.]);
    // +x lies on the sphere's equator at phi 0.
    let spherical = Mapping2D::Spherical(IDENTITY);
    let st = spherical.map(&Context {
        p: [2., 0., 0.],
        n: [0., 0., 1.],
        uv: [0., 0.],
    });
    assert!((st[0] - 0.5).abs() < 1e-6 && st[1].abs() < 1e-6, "{st:?}");
}

#[test]
fn operators_combine_their_operands() {
    let half = Arc::new(Texture::Constant([0.5; 3]));
    let scale = Texture::Scale {
        tex: Operand::Texture(half.clone()),
        scale: constant(0.5),
    };
    assert_eq!(scale.evaluate(&at(0., 0.)), [0.25; 3]);
    let mix = Texture::Mix {
        tex1: constant(0.),
        tex2: constant(1.),
        amount: constant(0.25),
    };
    assert_eq!(mix.evaluate(&at(0., 0.)), [0.25; 3]);
    // The preview plane's normal is +z: dir +z takes tex1 entirely.
    let direction = Texture::DirectionMix {
        tex1: constant(1.),
        tex2: constant(0.),
        dir: [0., 0., 1.],
    };
    assert_eq!(direction.evaluate(&at(0., 0.)), [1.; 3]);
    let bilerp = Texture::Bilerp {
        mapping: Mapping2D::Uv {
            su: 1.,
            sv: 1.,
            du: 0.,
            dv: 0.,
        },
        v00: [0.; 3],
        v01: [1.; 3],
        v10: [0.; 3],
        v11: [1.; 3],
    };
    assert_eq!(bilerp.evaluate(&at(0.5, 0.25)), [0.25; 3]);
}

#[test]
fn images_flip_t_wrap_scale_and_invert() {
    // 2×1: left black, right white.
    let image = Arc::new(Image {
        width: 2,
        height: 1,
        pixels: vec![[0.; 3], [1.; 3]],
    });
    let texture = |wrap, scale, invert| Texture::Image {
        mapping: Mapping2D::Uv {
            su: 1.,
            sv: 1.,
            du: 0.,
            dv: 0.,
        },
        image: image.clone(),
        wrap,
        scale,
        invert,
    };
    // Texel centers: s 0.25 is the left texel, 0.75 the right one.
    assert_eq!(
        texture(Wrap::Repeat, 1., false).evaluate(&at(0.25, 0.5)),
        [0.; 3]
    );
    assert_eq!(
        texture(Wrap::Repeat, 1., false).evaluate(&at(0.75, 0.5)),
        [1.; 3]
    );
    assert_eq!(
        texture(Wrap::Repeat, 2., false).evaluate(&at(0.75, 0.5)),
        [2.; 3]
    );
    assert_eq!(
        texture(Wrap::Repeat, 1., true).evaluate(&at(0.75, 0.5)),
        [0.; 3]
    );
    // Outside [0, 1): repeat wraps around, black is black.
    assert_eq!(
        texture(Wrap::Repeat, 1., false).evaluate(&at(1.75, 0.5)),
        [1.; 3]
    );
    assert_eq!(
        texture(Wrap::Black, 1., false).evaluate(&at(1.75, 0.5)),
        [0.; 3]
    );
    assert_eq!(
        texture(Wrap::Clamp, 1., false).evaluate(&at(1.75, 0.5)),
        [1.; 3]
    );
}

#[test]
fn procedurals_follow_r4_formulas() {
    // FBm and wrinkled are r4's fbm / turbulence at the mapped point.
    let fbm_texture = Texture::Fbm {
        texture_from_world: IDENTITY,
        octaves: 8,
        omega: 0.5,
    };
    let (p, _, f, t, _) = R4_NOISE[1];
    let ctx = Context {
        p,
        n: [0., 0., 1.],
        uv: [0., 0.],
    };
    assert_eq!(fbm_texture.evaluate(&ctx), [f; 3]);
    let wrinkled = Texture::Wrinkled {
        texture_from_world: IDENTITY,
        octaves: 8,
        omega: 0.5,
    };
    assert_eq!(wrinkled.evaluate(&ctx), [t; 3]);
    // Marble: r4 returns an albedo spectrum. Its sigmoid polynomial's
    // coefficients are recorded here. Near 550 nm
    // that spectrum follows this evaluator's green.
    const R4_MARBLE: [((f32, f32), [f64; 3]); 6] = [
        ((0.3, 0.0), [9.465643e-8, -2.1082795e-5, 0.6450306]),
        ((0.3, 0.37), [2.6623827e-6, -0.0020972246, 1.5451709]),
        ((0.3, 0.74), [1.3733004e-5, -0.015925432, 5.7034535]),
        ((0.3, 1.11), [1.45532695e-5, -0.016888885, 5.1267147]),
        ((0.3, 1.48), [1.3733334e-5, -0.015925813, 5.7036576]),
        ((0.8, 2.2), [1.3948843e-5, -0.016179109, 5.0908146]),
    ];
    let marble = Texture::Marble {
        texture_from_world: IDENTITY,
        octaves: 8,
        omega: 0.5,
        scale: 1.,
        variation: 0.2,
    };
    for ((u, v), [c0, c1, c2]) in R4_MARBLE {
        let value = marble.evaluate(&at(u, v));
        let x = (c0 * 550. + c1) * 550. + c2;
        let r4 = 0.5 + x / (2. * (1. + x * x).sqrt());
        assert!(
            (value[1] as f64 - r4).abs() < 0.04,
            "({u}, {v}): {value:?} vs {r4}"
        );
    }
    // Dots: cells are centred on integer st; near the centres, some cells
    // hold a dot and some do not.
    let dots = Texture::Dots {
        mapping: Mapping2D::Uv {
            su: 8.,
            sv: 8.,
            du: 0.,
            dv: 0.,
        },
        inside: constant(1.),
        outside: constant(0.),
    };
    let values = (0..64)
        .map(|i| {
            dots.evaluate(&at(
                (i % 8) as f32 / 8. + 0.001,
                (i / 8) as f32 / 8. + 0.001,
            ))[0]
        })
        .collect::<Vec<_>>();
    assert!(values.contains(&1.) && values.contains(&0.), "{values:?}");
}

#[test]
fn thumbnails_are_srgb_rgba_rows_from_the_top() {
    // Left half black, right half white: a 2×2 thumbnail of a u-step.
    let step = Texture::Checkerboard2D {
        mapping: Mapping2D::Uv {
            su: 2.,
            sv: 0.,
            du: 0.,
            dv: 0.,
        },
        tex1: constant(0.),
        tex2: constant(1.),
    };
    let rgba = thumbnail(&step, 2);
    assert_eq!(rgba.len(), 16);
    assert_eq!(&rgba[0..4], &[0, 0, 0, 255]);
    assert_eq!(&rgba[4..8], &[255, 255, 255, 255]);
    assert_eq!(srgb8(0.5), 188, "linear 0.5 is sRGB 188");
    assert_eq!(srgb8(2.), 255, "clamped (ruling D)");
    // Row 0 is v near 1.
    assert!(preview_context(0, 0, 4).uv[1] > 0.8);
}
