#![allow(non_snake_case)]

use log::warn;
use std::f32::consts::PI;

/// Square function
fn sqr(x: f32) -> f32 {
    x * x
}

/// G function for sphere table computation
fn G(w: f32, s: f32, g: f32) -> f32 {
    -2.0 * w.sin() * s.cos() * g.cos() + PI / 2.0 - g + g.sin() * g.cos()
}

/// H function for sphere table computation
fn H(w: f32, s: f32, g: f32) -> f32 {
    let sins_sq = sqr(s.sin());
    let cosg_sq = sqr(g.cos());

    w.cos() * (g.cos() * (sins_sq - cosg_sq).sqrt() + sins_sq * (g.cos() / s.sin()).asin())
}

/// Compute projected (cosine-weighted) solid angle of spherical cap clipped to hemisphere
fn ihemi(w: f32, s: f32) -> f32 {
    let g = (s.cos() / w.sin()).asin();
    let sins_sq = sqr(s.sin());

    if w >= 0.0 && w <= (PI / 2.0 - s) {
        PI * w.cos() * sins_sq
    } else if w >= (PI / 2.0 - s) && w < PI / 2.0 {
        PI * w.cos() * sins_sq + G(w, s, g) - H(w, s, g)
    } else if w >= PI / 2.0 && w < (PI / 2.0 + s) {
        G(w, s, g) + H(w, s, g)
    } else {
        0.0
    }
}

/// Generate sphere table for LTC fitting
///
/// This function computes a lookup table for the projected solid angle of spherical caps,
/// which is used in LTC (Linearly Transformed Cosines) calculations.
///
/// # Arguments
/// * `width` - Width of the table
/// * `height` - Height of the table
///
/// # Returns
/// * `Vec<f32>` - Flattened width*height table of sphere values
pub fn gen_sphere_tab(width: usize, height: usize) -> Vec<f32> {
    let mut tab_sphere = vec![0.0; width * height];

    for j in 0..height {
        for i in 0..width {
            let U1 = i as f32 / (width - 1) as f32;
            let U2 = j as f32 / (height - 1) as f32;

            // z = cos(elevation angle)
            let z = 2.0 * U1 - 1.0;

            // length of average dir., proportional to sin(sigma)^2
            let len = U2;

            let sigma = len.sqrt().asin();
            let omega = z.acos();

            // compute projected (cosine-weighted) solid angle of spherical cap
            let value = if sigma > 0.0 {
                ihemi(omega, sigma) / (PI * len)
            } else {
                z.max(0.0)
            };

            if value.is_nan() {
                warn!("NaN value encountered at ({}, {})", i, j);
            }

            tab_sphere[i + j * width] = value;
        }
    }

    tab_sphere
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqr() {
        assert_eq!(sqr(2.0), 4.0);
        assert_eq!(sqr(-3.0), 9.0);
        assert_eq!(sqr(0.0), 0.0);
    }

    #[test]
    fn test_gen_sphere_tab_size() {
        let width = 8;
        let height = 8;
        let tab = gen_sphere_tab(width, height);
        assert_eq!(tab.len(), width * height);
    }

    #[test]
    fn test_gen_sphere_tab_no_nan() {
        let width = 16;
        let height = 16;
        let tab = gen_sphere_tab(width, height);
        for (i, &value) in tab.iter().enumerate() {
            assert!(!value.is_nan(), "NaN value at index {}", i);
        }
    }

    #[test]
    fn test_gen_sphere_tab_values_in_range() {
        let width = 16;
        let height = 16;
        let tab = gen_sphere_tab(width, height);
        for (i, &value) in tab.iter().enumerate() {
            assert!(value >= 0.0, "Negative value {} at index {}", value, i);
            assert!(
                value.is_finite(),
                "Non-finite value {} at index {}",
                value,
                i
            );
        }
    }
}
