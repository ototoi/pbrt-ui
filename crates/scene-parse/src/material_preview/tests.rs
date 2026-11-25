use super::*;

fn constant(value: f32) -> Operand {
    Operand::Constant([value; 3])
}

/// The hit at the sphere's centre pixel, facing the eye.
fn centre() -> Hit {
    hit_at(0., 0., 0.01).unwrap()
}

#[test]
fn the_sphere_is_pbrts_seen_from_minus_y() {
    let hit = centre();
    assert_eq!(hit.ctx.p, [0., -1., 0.]);
    assert_eq!(hit.ctx.n, hit.ctx.p);
    assert_eq!(hit.cos_view, 1.);
    // φ = 3π/2 on -y, θ = π/2 on the equator.
    assert_eq!(hit.ctx.uv, [0.75, 0.5]);
    // The top pole is v = 1, the bottom v = 0 (pbrt's 1 − θ/π).
    assert!((hit_at(0., 0.999, 0.01).unwrap().ctx.uv[1] - 1.).abs() < 0.03);
    assert!(hit_at(0., -0.999, 0.01).unwrap().ctx.uv[1] < 0.03);
    assert_eq!(hit_at(0.8, 0.8, 0.01), None, "outside the silhouette");
}

#[test]
fn the_gbuffer_is_computed_once_per_size() {
    let a = gbuffer(16);
    let b = gbuffer(16);
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.len(), 256);
    // Corners are backdrop, the centre is sphere.
    assert_eq!(a[0], None);
    assert!(a[8 * 16 + 8].is_some());
    // Row 0 is the top: the upper half has z > 0.
    assert!(a[4 * 16 + 8].unwrap().ctx.p[2] > 0.);
}

#[test]
fn fresnel_matches_r4() {
    // Normal incidence: ((η − 1) / (η + 1))², and ((η−1)² + k²) / ((η+1)² + k²).
    assert!((fr_dielectric(1., 1.5) - 0.04).abs() < 1e-6);
    assert_eq!(
        fr_dielectric(0.1, 1. / 1.5),
        1.,
        "total internal reflection"
    );
    let (eta, k) = (0.24588375, 3.3783503);
    let expected = ((eta - 1.) * (eta - 1.) + k * k) / ((eta + 1.) * (eta + 1.) + k * k);
    assert!((fr_complex(1., eta, k) - expected).abs() < 1e-5);
    // Grazing incidence reflects everything.
    assert!((fr_complex(0., eta, k) - 1.).abs() < 1e-5);
}

#[test]
fn diffuse_follows_its_reflectance_and_textures() {
    let hit = centre();
    let grey = Material::Diffuse {
        reflectance: constant(0.5),
    };
    let bright = Material::Diffuse {
        reflectance: constant(1.),
    };
    let (g, b) = (grey.shade(&hit), bright.shade(&hit));
    assert!(g[0] > 0. && (b[0] - 2. * g[0]).abs() < 1e-6, "{g:?} {b:?}");
    // A textured reflectance reads the sphere's uv.
    let textured = Material::Diffuse {
        reflectance: Operand::Texture(Arc::new(crate::texture_eval::Texture::Bilerp {
            mapping: crate::texture_eval::Mapping2D::Uv {
                su: 1.,
                sv: 1.,
                du: 0.,
                dv: 0.,
            },
            v00: [0.; 3],
            v01: [0.; 3],
            v10: [1.; 3],
            v11: [1.; 3],
        })),
    };
    // u = 0.75 at the centre: three quarters of the way to v10.
    assert!((textured.shade(&hit)[0] - 0.75 * b[0]).abs() < 1e-5);
}

#[test]
fn copper_reflects_red_more_than_blue() {
    let copper = Material::Conductor {
        tint: Tint::Fresnel {
            eta: Operand::Constant([0.24588375, 1.072285, 1.156862]),
            k: Operand::Constant([3.3783503, 2.5907342, 2.4572217]),
        },
        roughness: Roughness::smooth(),
    };
    let c = copper.shade(&centre());
    assert!(c[0] > c[1] && c[1] > c[2] * 0.9, "{c:?}");
    // r4's reflectance form: reflectance r reflects r at normal incidence.
    let tinted = Material::Conductor {
        tint: Tint::Reflectance(Operand::Constant([0.9, 0.5, 0.2])),
        roughness: Roughness::smooth(),
    };
    let (eta, k) = conductor_ior(
        &Tint::Reflectance(Operand::Constant([0.9, 0.5, 0.2])),
        &centre().ctx,
        1.,
    );
    for (c, r) in [0.9, 0.5, 0.2].into_iter().enumerate() {
        assert!((fr_complex(1., eta[c], k[c]) - r).abs() < 1e-4);
    }
    let t = tinted.shade(&centre());
    assert!(t[0] > t[1] && t[1] > t[2], "{t:?}");
}

#[test]
fn roughness_blurs_reflections_and_widens_the_highlight() {
    let hit = centre();
    let sharp = reflection(&hit, 0.);
    let rough = reflection(&hit, 1.);
    assert_eq!(rough, SKY_MEAN);
    assert_ne!(sharp, rough);
    // A rough lobe is lower at its peak but reaches further.
    let n = normalize(add(key_direction(), VIEW));
    let off = normalize([n[0] + 0.3, n[1], n[2]]);
    assert!(highlight(n, 0.05).0 > highlight(n, 0.5).0);
    assert!(highlight(off, 0.05).0 < highlight(off, 0.5).0);
}

#[test]
fn glass_shows_the_refracted_backdrop_and_interface_is_invisible() {
    let size = 64;
    let pixels = gbuffer(size);
    let glass = Material::Dielectric {
        eta: 1.5,
        roughness: Roughness::smooth(),
        thin: false,
    };
    let interface = Material::Interface;
    let mut differs = false;
    for (i, hit) in pixels.iter().enumerate() {
        let Some(hit) = hit else { continue };
        let i = i as u32;
        let behind = backdrop(i % size, i / size, size);
        assert_eq!(interface.shade(hit), behind, "interface pixel {i}");
        differs |= glass.shade(hit) != behind;
        // Glass passes most light at the centre: values stay in the
        // backdrop's range, not a diffuse grey.
        if i == size / 2 * size + size / 2 {
            let g = glass.shade(hit);
            assert!(g[0] > 0.1 && g[0] < 0.4, "{g:?}");
        }
    }
    assert!(differs, "refraction moves the checker");
    // A thin sheet does not bend light: only Fresnel dims the backdrop.
    let thin = Material::Dielectric {
        eta: 1.5,
        roughness: Roughness::smooth(),
        thin: true,
    };
    let hit = centre();
    let t = thin.shade(&hit);
    let behind = environment(hit.ctx.p, [0., 1., 0.]);
    assert!(t[0] < behind[0] + reflection(&hit, 0.)[0] * 0.1 + 0.05);
}

#[test]
fn mix_blends_its_materials_by_amount() {
    let hit = centre();
    let black = Arc::new(Material::Diffuse {
        reflectance: constant(0.),
    });
    let white = Arc::new(Material::Diffuse {
        reflectance: constant(1.),
    });
    let mix = |amount| Material::Mix {
        materials: [black.clone(), white.clone()],
        amount: constant(amount),
    };
    assert_eq!(mix(0.).shade(&hit), black.shade(&hit));
    assert_eq!(mix(1.).shade(&hit), white.shade(&hit));
    let half = mix(0.25).shade(&hit)[0];
    assert!((half - 0.25 * white.shade(&hit)[0]).abs() < 1e-6);
}

#[test]
fn coats_add_reflection_over_their_base() {
    let hit = hit_at(0.7, 0., 0.01).unwrap();
    let base = Material::Diffuse {
        reflectance: constant(0.2),
    };
    let coated = Material::CoatedDiffuse {
        reflectance: constant(0.2),
        roughness: Roughness::smooth(),
        eta: 1.5,
    };
    let (b, c) = (base.shade(&hit), coated.shade(&hit));
    assert!(
        c[0] > b[0],
        "a glossy coat brightens toward the rim: {b:?} {c:?}"
    );
}

#[test]
fn thumbnails_are_rgba_with_the_backdrop_around_the_sphere() {
    let rgba = thumbnail(
        &Material::Diffuse {
            reflectance: constant(0.5),
        },
        8,
    );
    assert_eq!(rgba.len(), 8 * 8 * 4);
    let corner = backdrop(0, 0, 8).map(srgb8);
    assert_eq!(&rgba[0..4], &[corner[0], corner[1], corner[2], 255]);
    assert_ne!(&rgba[0..3], &rgba[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 3]);
}

#[test]
fn roughness_past_one_stays_finite() {
    // r4's roughness_to_alpha is a bare sqrt(max(r, 0)) - no ceiling - so
    // the wire accepts roughness past 1. The preview's Blinn-Phong
    // exponent goes negative there: dot(n, h).max(0.) == 0 on the rim,
    // 0^-x is infinite, and the zero key cosine turns it into NaN, which
    // the PNG writes as black pixels.
    let shiny = Material::Conductor {
        tint: Tint::Reflectance(Operand::Constant([0.9, 0.5, 0.2])),
        roughness: Roughness {
            value: constant(1.1),
            remap: false,
        },
    };
    for hit in gbuffer(64).iter().flatten() {
        for channel in shiny.shade(hit) {
            assert!(channel.is_finite(), "roughness 1.1 went nonfinite");
        }
    }
}

#[test]
fn the_geometry_cache_stays_within_its_budget() {
    // One buffer per supported size (8..=512) would retain ~1.8 GiB of
    // Option<Hit> in the host process; the
    // cache bounds itself in bytes and rebuilds whatever fell out.
    let mut cache = Geometry::default();
    for size in (8..=512u32).step_by(8) {
        cache.get(size);
    }
    assert!(cache.bytes <= GEOMETRY_BUDGET, "{}", cache.bytes);
    assert!(
        cache.map.len() < 64,
        "the least recently used sizes must fall out: {}",
        cache.map.len()
    );
    // What stayed retained is reused, not rebuilt.
    let last = cache.get(512);
    assert!(Arc::ptr_eq(&last, &cache.map[&512]));
}
