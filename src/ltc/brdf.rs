#![allow(non_snake_case)]

pub trait Brdf {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32); //(value, pdf)
    fn fresnel(&self, V: &glam::Vec3, L: &glam::Vec3) -> f32 {
        let H = (*V + *L).normalize();
        (1.0 - V.dot(H).max(0.0)).powi(5)   //Schlick's approximation
    }
    fn sample(&self, V: &glam::Vec3, alpha: f32, U1: f32, U2: f32) -> glam::Vec3; //L
}
