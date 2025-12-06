#![allow(non_snake_case)]

use super::brdf::Brdf;
use std::sync::Arc;

pub trait MicrofacetDistribution {
    fn d(&self, wh: &glam::Vec3) -> f32;
    fn lambda(&self, w: &glam::Vec3) -> f32;
    fn g1(&self, w: &glam::Vec3) -> f32 {
        return 1.0 / (1.0 + self.lambda(w));
    }
    fn g(&self, wo: &glam::Vec3, wi: &glam::Vec3) -> f32 {
        return 1.0 / (1.0 + self.lambda(wo) + self.lambda(wi));
    }
    fn sample_wh(&self, wo: &glam::Vec3, u: &glam::Vec2) -> glam::Vec3;
    fn pdf(&self, wo: &glam::Vec3, wh: &glam::Vec3) -> f32;
}

#[derive(Debug, Clone)]
pub struct TrowbridgeReitzDistribution {
    alphax: f32,
    alphay: f32,
    samplevis: bool,
}

impl MicrofacetDistribution for TrowbridgeReitzDistribution {
    fn d(&self, wh: &glam::Vec3) -> f32 {
        //TODO
        0.0
    }

    fn lambda(&self, w: &glam::Vec3) -> f32 {
        //TODO
        0.0
    }

    fn sample_wh(&self, wo: &glam::Vec3, u: &glam::Vec2) -> glam::Vec3 {
        //TODO
        glam::Vec3::ZERO
    }

    fn pdf(&self, wo: &glam::Vec3, wh: &glam::Vec3) -> f32 {
        //TODO
        0.0
    }
}

/* 
pub trait MicrofacetDistributionFactory {
    fn create(&self, alpha: f32) -> Arc<dyn MicrofacetDistribution>;
}

pub struct TrowbridgeReitzDistributionFactory;

impl MicrofacetDistributionFactory for TrowbridgeReitzDistributionFactory {
    fn create(&self, alpha: f32) -> Arc<dyn MicrofacetDistribution> {
        Arc::new(TrowbridgeReitzDistribution {
            alphax: alpha,
            alphay: alpha,
            samplevis: true,
        })
    }
}


#[derive(Clone)]
pub struct BrdfMicrofacetReflection {
    pub factory: Arc<dyn MicrofacetDistributionFactory>,
}


impl BrdfMicrofacetReflection {
    pub fn new(factory: &Arc<dyn MicrofacetDistributionFactory>) -> Self {
        Self {
            factory: factory.clone(),
        }
    }
}
*/

#[derive(Clone, Copy, Debug, Default)]
pub struct BrdfMicrofacetReflection;

impl BrdfMicrofacetReflection {
    pub fn new() -> Self {
        Self::default()
    }
}

fn create_distribution(alpha: f32) -> Arc<dyn MicrofacetDistribution> {
    Arc::new(TrowbridgeReitzDistribution {
        alphax: alpha,
        alphay: alpha,
        samplevis: true,
    })
}

impl Brdf for BrdfMicrofacetReflection {
    fn eval(&self, V: &glam::Vec3, L: &glam::Vec3, alpha: f32) -> (f32, f32) {
        //TODO
        let distribution = create_distribution(alpha);

        //TODO
        (0.0, 0.0)
    }
    fn sample(&self, V: &glam::Vec3, alpha: f32, U1: f32, U2: f32) -> glam::Vec3 {
        //TODO
        glam::Vec3::ZERO
    }
}
