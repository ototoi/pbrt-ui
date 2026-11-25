// Lit fragment: emitters show their radiance, everything else is shaded
// by its lobe over the light list. Output is linear HDR; `output.wgsl` exposes it.
struct Fragment {
    @location(0) color: vec4<f32>,
    @location(1) pick: u32,
};

fn emitted(appearance: Lobe, i: Out) -> vec3<f32> {
    let two_sided = appearance.texture.y > 0.5;
    let facing = dot(normalize(i.normal), -normalize(i.world)) > 0.0;
    if two_sided || facing {
        return appearance.diffuse;
    }
    return vec3<f32>(0.0);
}

// Fraction of a light's energy that crosses a smooth coat on its way in:
// 1 − Schlick Fresnel along the light for point, spot and distant lights,
// 1 − the hemispherical reflectance r_e for area and environment lights.
// `coat`: [F0, alpha, eta, internal reflectance].
fn coat_entry(light: Light, coat: vec4<f32>, p: vec3<f32>, n: vec3<f32>) -> f32 {
    var l = vec3<f32>(0.0);
    switch light.kind.x {
        case LIGHT_POINT, LIGHT_SPOT, LIGHT_GONIOMETRIC, LIGHT_PROJECTION: { l = normalize(light.position.xyz - p); }
        case LIGHT_DISTANT: { l = light.direction.xyz; }
        default: {
            // r_i = 1 − (1 − r_e)/η²  ⇒  r_e = 1 − η²(1 − r_i).
            let outside = 1.0 - coat.z * coat.z * (1.0 - coat.w);
            return 1.0 - outside;
        }
    }
    let cos_l = clamp(dot(n, l), 0.0, 1.0);
    return 1.0 - (coat.x + (1.0 - coat.x) * pow(1.0 - cos_l, 5.0));
}

// Lobe shading: Lambert diffuse plus a GGX specular layer, under an
// optional dielectric coat. Light under the coat crosses it twice (in and
// out); the diffuse layer also bounces inside it, which gives the smooth
// coated-Lambertian form ρ·T_in·T_out / (η²(1 − ρ·r_i)).
// `ao`: the environment light's screen-space AO at this pixel (1 for glass
// and anything else outside PreZ). It scales the environment's diffuse,
// specular and coat alike: against r4, Lagarde's specular occlusion came
// out within 0.001 of plain AO (`tests/lit_ao_reference/README.md`). Other
// lights keep their shadow maps.
fn surface(lobe: Lobe, i: Out, ao: f32) -> vec3<f32> {
    let albedo = lobe.diffuse;
    // pbrt's diffuse reflection works on whichever side is seen: face the
    // shading normal toward the eye (points are eye-relative).
    var n = normalize(i.normal);
    let v = normalize(-i.world);
    if dot(n, v) < 0.0 {
        n = -n;
    }
    let has_specular = lobe.spec_alpha > 0.0 && any(lobe.spec_f0 > vec3<f32>(0.0));
    let has_coat = lobe.coat.x > 0.0;
    let specular_lobe = ltc_lobe(max(lobe.spec_alpha, 1e-4), n, v);
    let coat_lobe = ltc_lobe(max(lobe.coat.y, 1e-4), n, v);
    let coat_f0 = vec3<f32>(lobe.coat.x);
    var irradiance = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    var coat = vec3<f32>(0.0);
    for (var index = 0u; index < view.mode.y; index = index + 1u) {
        let visibility = shadow_visibility(index, i.world, n);
        if visibility <= 0.0 {
            continue;
        }
        let light = lights[index];
        let environment = light.kind.x == LIGHT_INFINITE || light.kind.x == LIGHT_ENVIRONMENT;
        let occluded = visibility * select(1.0, ao, environment);
        var entry = 1.0;
        if has_coat {
            entry = coat_entry(light, lobe.coat, i.world, n);
            coat += light_specular(light, coat_lobe, coat_f0, i.world, n, v) * occluded;
        }
        irradiance += light_irradiance(light, i.world, n) * (occluded * entry);
        if has_specular {
            specular += light_specular(light, specular_lobe, lobe.spec_f0, i.world, n, v) * (occluded * entry);
        }
    }
    var diffuse = albedo / PI * irradiance;
    if has_coat {
        let exit = vec3<f32>(1.0) - ltc_albedo(coat_lobe, coat_f0);
        let eta2 = lobe.coat.z * lobe.coat.z;
        diffuse = diffuse * exit / (eta2 * (vec3<f32>(1.0) - albedo * lobe.coat.w));
        // Of the light the layer under the coat reflects, the share e inside
        // the critical cone gets out; the rest is totally reflected back
        // down, where the layer reflects its albedo A again: the escaping
        // total is the series e·(1 + A(1−e) + …) = e / (1 − A(1 − e)).
        let cos_critical = sqrt(max(1.0 - 1.0 / eta2, 0.0));
        let escape = ltc_cone_fraction(specular_lobe, n, cos_critical);
        let layer_albedo = ltc_albedo(specular_lobe, lobe.spec_f0);
        let trapped = vec3<f32>(1.0) - layer_albedo * (1.0 - escape);
        specular = specular * exit * escape / max(trapped, vec3<f32>(1e-3));
    }
    return diffuse + specular + coat;
}

// F0 of a metal given as `reflectance` r: pbrt turns it into eta 1 and
// k = 2·sqrt(r)/sqrt(1 − r), divided by the index of the medium around it.
fn reflectance_f0(r: vec3<f32>, medium: f32) -> vec3<f32> {
    let clamped = clamp(r, vec3<f32>(0.0), vec3<f32>(1.0));
    let k = 2.0 * sqrt(clamped) / sqrt(max(vec3<f32>(1.0) - clamped, vec3<f32>(1e-6))) / medium;
    let eta = 1.0 / medium;
    return ((eta - 1.0) * (eta - 1.0) + k * k) / ((eta + 1.0) * (eta + 1.0) + k * k);
}

// GGX alpha from evaluated u and v roughness, as `alpha_of` does on the CPU.
fn program_alpha(u: vec3<f32>, v: vec3<f32>, remap: bool) -> f32 {
    var rough = max(vec2<f32>(u.x, v.x), vec2<f32>(0.0));
    if remap {
        rough = sqrt(rough);
    }
    return max((rough.x + rough.y) * 0.5, 1e-5);
}

// The lobe with its texture inputs evaluated at this pixel. One program call
// site: drivers inline every call, and each one multiplies the compile time.
fn resolved_lobe(base: Lobe, point: TexturePoint) -> Lobe {
    var programs = array<u32, 5>(
        base.programs_a.x, base.programs_a.y, base.programs_a.z, base.programs_a.w, base.programs_b.x,
    );
    var values: array<vec3<f32>, 5>;
    for (var k = 0u; k < 5u; k = k + 1u) {
        if programs[k] != 0u {
            values[k] = program_value(programs[k], point);
        }
    }
    var lobe = base;
    let remap = (base.flags.x & LOBE_REMAP_ROUGHNESS) != 0u;
    if programs[0] != 0u {
        if (base.flags.x & LOBE_REFLECTANCE_IS_F0) != 0u {
            let medium = select(1.0, base.coat.z, base.coat.x > 0.0);
            lobe.spec_f0 = reflectance_f0(values[0], medium);
        } else {
            lobe.diffuse = values[0];
        }
    }
    // r4 clamps a diffuse reflectance; marble's spline and noise overshoot it.
    lobe.diffuse = clamp(lobe.diffuse, vec3<f32>(0.0), vec3<f32>(1.0));
    if programs[1] != 0u {
        lobe.spec_alpha = program_alpha(values[1], values[2], remap);
    }
    if programs[3] != 0u {
        lobe.coat.y = program_alpha(values[3], values[4], remap);
    }
    return lobe;
}

// One non-mix material: radiance, and how opaque it is at this pixel.
fn shade_leaf(base: Lobe, i: Out, point: TexturePoint, ao: f32) -> vec4<f32> {
    if base.kind == KIND_INVISIBLE {
        return vec4<f32>(0.0);
    }
    if base.kind == KIND_EMISSIVE && view.mode.w != 1u {
        return vec4<f32>(emitted(base, i), 1.0);
    }
    let lobe = resolved_lobe(base, point);
    if view.mode.w == 1u {
        return vec4<f32>(lobe.diffuse, 1.0 - lobe.transmission);
    }
    return vec4<f32>(surface(lobe, i, ao), 1.0 - lobe.transmission);
}

// pbrt's mix picks a child with probability `amount` per ray, so its expected
// radiance is the amount-weighted sum of the children's. Mixes nest, and WGSL
// has no recursion: walk them with a stack. lit::evaluate bounds the leaves
// and rejects cycles.
fn shade_material(root: u32, i: Out, point: TexturePoint, ao: f32) -> vec4<f32> {
    var nodes: array<u32, 16>;
    var weights: array<f32, 16>;
    nodes[0] = root;
    weights[0] = 1.0;
    var depth = 1u;
    var total = vec4<f32>(0.0);
    for (var steps = 0u; depth > 0u && steps < 32u; steps += 1u) {
        depth -= 1u;
        let weight = weights[depth];
        // A draw without a GPU-resident lobe shades as pbrt's default material.
        var lobe: Lobe;
        lobe.diffuse = DEFAULT_ALBEDO;
        if nodes[depth] < view.mode.z {
            lobe = appearances[nodes[depth]];
        }
        if lobe.kind != KIND_MIX {
            total += weight * shade_leaf(lobe, i, point, ao);
            continue;
        }
        let amount = clamp(program_value(lobe.programs_b.y, point).x, 0.0, 1.0);
        if amount < 1.0 {
            nodes[depth] = lobe.programs_b.z;
            weights[depth] = weight * (1.0 - amount);
            depth += 1u;
        }
        if amount > 0.0 {
            nodes[depth] = lobe.programs_b.w;
            weights[depth] = weight * amount;
            depth += 1u;
        }
    }
    return total;
}

fn shade_draw(i: Out, point: TexturePoint, ao: f32) -> vec4<f32> {
    // Cutout at the coverage's majority: pbrt decides partial alpha per ray, and
    // filtered edges of a binary mask are partial too. lit::evaluate notes it.
    if i.alpha != 0u && program_value(i.alpha, point).x < 0.5 {
        discard;
    }
    if i.appearance < view.mode.z && appearances[i.appearance].kind == KIND_INVISIBLE {
        discard;
    }
    return shade_material(i.appearance, i, point, ao);
}

fn texture_point(i: Out) -> TexturePoint {
    return TexturePoint(
        i.uv, dpdx(i.uv), dpdy(i.uv), normalize(i.normal),
        i.world, dpdx(i.world), dpdy(i.world),
    );
}

@fragment fn fs_lit(i: Out) -> Fragment {
    let point = texture_point(i);
    var out: Fragment;
    let ao = textureLoad(ao_map, vec2<i32>(i.clip.xy), 0).x;
    out.color = vec4<f32>(shade_draw(i, point, ao).rgb, 1.0);
    out.pick = i.pick;
    return out;
}

// Glass: its reflection, premultiplied, with alpha = 1 − transmission so
// the blend lets the transmitted share of what is behind through (no
// refraction).
@fragment fn fs_lit_transparent(i: Out) -> Fragment {
    let point = texture_point(i);
    var out: Fragment;
    // Glass and transmitting mixes are outside PreZ: no AO.
    out.color = shade_draw(i, point, 1.0);
    // Nothing here at all, as where a mix selects an invisible material: leave
    // the pick and depth to what is behind.
    if out.color.a <= 0.0 {
        discard;
    }
    out.pick = i.pick;
    return out;
}

// PreZ: the depth the opaque Lit draw then tests `Equal`, and the eye-facing
// interpolated normal (world axes, Rgb10a2Unorm) for screen-space AO. Draws
// without alpha use `fs_prez` and skip the texture VM.
// The facet decides which side faces the eye, not the interpolated normal,
// which leans away at a tessellated silhouette.
fn prez_normal(i: Out, facet: vec3<f32>) -> vec4<f32> {
    var n = normalize(i.normal);
    if dot(n, facet) * dot(facet, -i.world) < 0.0 {
        n = -n;
    }
    return vec4<f32>(n * 0.5 + 0.5, 1.0);
}

fn prez_facet(i: Out) -> vec3<f32> {
    return cross(dpdx(i.world), dpdy(i.world));
}

@fragment fn fs_prez(i: Out) -> @location(0) vec4<f32> {
    return prez_normal(i, prez_facet(i));
}

@fragment fn fs_prez_alpha(i: Out) -> @location(0) vec4<f32> {
    let facet = prez_facet(i);
    if program_value(i.alpha, texture_point(i)).x < 0.5 {
        discard;
    }
    return prez_normal(i, facet);
}
