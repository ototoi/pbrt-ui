#![allow(non_snake_case)]

use glam::{Mat3, Vec2, Vec4};

/// Pack LTC data into two texture-friendly tables
///
/// Based on fitLTC.cpp's packTab function (line 355):
/// - tex1: Inverse matrix parameters (invM[0][0], invM[0][2], invM[2][0], invM[2][2])
/// - tex2: (magnitude, fresnel, 0.0, sphere_value)
///
/// # Arguments
/// * `tab` - Vector of Mat3 inverse matrices from LTC fitting (already inverted)
/// * `tab_mag_fresnel` - Vector of Vec2 containing (magnitude, fresnel) values
/// * `tab_sphere` - Vector of f32 containing sphere table values
/// * `width` - Width of the table
/// * `height` - Height of the table
///
/// # Returns
/// * Tuple of (Vec<Vec4>, Vec<Vec4>) for tex1 and tex2
///
/// # Panics
/// * Panics if any input vector length != width * height
pub fn pack_tab(
    tab: Vec<Mat3>,
    tab_mag_fresnel: Vec<Vec2>,
    tab_sphere: Vec<f32>,
    width: usize,
    height: usize,
) -> (Vec<Vec4>, Vec<Vec4>) {
    let expected_len = width * height;
    assert_eq!(
        tab.len(),
        expected_len,
        "tab must have width * height elements"
    );
    assert_eq!(
        tab_mag_fresnel.len(),
        expected_len,
        "tab_mag_fresnel must have width * height elements"
    );
    assert_eq!(
        tab_sphere.len(),
        expected_len,
        "tab_sphere must have width * height elements"
    );

    let mut tex1 = vec![Vec4::ZERO; expected_len];
    let mut tex2 = vec![Vec4::ZERO; expected_len];

    for i in 0..expected_len {
        let invM = tab[i]; // Already inverse matrix from fit_tab
        let mag_fresnel = tab_mag_fresnel[i];

        // Pack inverse matrix into tex1
        // tex1: (invM[0][0], invM[0][2], invM[2][0], invM[2][2])
        tex1[i] = Vec4::new(
            invM.col(0).x, // invM[0][0]
            invM.col(2).x, // invM[0][2]
            invM.col(0).z, // invM[2][0]
            invM.col(2).z, // invM[2][2]
        );

        // tex2: (magnitude, fresnel, 0.0, sphere_value)
        tex2[i] = Vec4::new(
            mag_fresnel.x, // magnitude
            mag_fresnel.y, // fresnel
            0.0,           // reserved
            tab_sphere[i], // sphere table value
        );
    }

    (tex1, tex2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_tab_size() {
        let width = 64;
        let height = 64;
        let tab = vec![Mat3::IDENTITY; width * height];
        let tab_mag_fresnel = vec![Vec2::new(1.0, 0.5); width * height];
        let tab_sphere = vec![0.75; width * height];

        let (tex1, tex2) = pack_tab(tab, tab_mag_fresnel, tab_sphere, width, height);

        assert_eq!(tex1.len(), width * height);
        assert_eq!(tex2.len(), width * height);
    }

    #[test]
    fn test_pack_tab_identity() {
        let width = 64;
        let height = 64;
        let tab = vec![Mat3::IDENTITY; width * height];
        let tab_mag_fresnel = vec![Vec2::new(1.0, 0.5); width * height];
        let tab_sphere = vec![0.75; width * height];

        let (tex1, tex2) = pack_tab(tab, tab_mag_fresnel, tab_sphere, width, height);

        // For identity matrix, invM = M = identity, so invM[0][0] = 1, invM[2][2] = 1, all off-diagonals = 0
        assert_eq!(tex1[0].x, 1.0); // invM[0][0]
        assert_eq!(tex1[0].y, 0.0); // invM[0][2]
        assert_eq!(tex1[0].z, 0.0); // invM[2][0]
        assert_eq!(tex1[0].w, 1.0); // invM[2][2]

        assert_eq!(tex2[0].x, 1.0); // magnitude
        assert_eq!(tex2[0].y, 0.5); // fresnel
        assert_eq!(tex2[0].z, 0.0); // reserved
        assert_eq!(tex2[0].w, 0.75); // sphere value
    }
}
