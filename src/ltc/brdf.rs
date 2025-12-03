#![allow(non_snake_case)]

pub trait Brdf {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32); //(value, pdf)
    fn sample(&self, V: &glam::Vec3, U1: f32, U2: f32, alpha: f32) -> glam::Vec3; //L
}
