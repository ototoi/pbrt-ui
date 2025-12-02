#![allow(non_snake_case)]

use super::parameters::N;
use glam::{Mat3, Vec2, Vec4};

/// Pack LTC data into two texture-friendly tables
/// 
/// Based on fitLTC.cpp's packTab function, this packs:
/// - tex1: Matrix parameters (M[0][0], M[0][2], M[1][1], M[1][2])
/// - tex2: Matrix parameters (M[2][0], M[2][2], magnitude, fresnel)
/// 
/// # Arguments
/// * `tab` - Vector of Mat3 matrices from LTC fitting
/// * `tab_mag_fresnel` - Vector of Vec2 containing (magnitude, fresnel) values
/// 
/// # Returns
/// * Tuple of (Vec<Vec4>, Vec<Vec4>) for tex1 and tex2
pub fn pack_tab(tab: Vec<Mat3>, tab_mag_fresnel: Vec<Vec2>) -> (Vec<Vec4>, Vec<Vec4>) {
    let mut tex1 = vec![Vec4::ZERO; N * N];
    let mut tex2 = vec![Vec4::ZERO; N * N];

    for i in 0..(N * N) {
        let M = tab[i];
        let mag_fresnel = tab_mag_fresnel[i];

        // Pack matrix and magnitude/fresnel into two Vec4s
        // tex1: (M[0][0], M[0][2], M[1][1], M[1][2])
        tex1[i] = Vec4::new(
            M.col(0).x,  // M[0][0]
            M.col(2).x,  // M[0][2]
            M.col(1).y,  // M[1][1]
            M.col(2).y,  // M[1][2]
        );

        // tex2: (M[2][0], M[2][2], magnitude, fresnel)
        tex2[i] = Vec4::new(
            M.col(0).z,          // M[2][0]
            M.col(2).z,          // M[2][2]
            mag_fresnel.x,       // magnitude
            mag_fresnel.y,       // fresnel
        );
    }

    (tex1, tex2)
}
