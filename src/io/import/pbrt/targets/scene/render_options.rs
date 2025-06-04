use crate::models::base::PropertyMap;

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub filter_name: String,
    pub filter_params: PropertyMap,
    pub film_name: String,
    pub film_params: PropertyMap,
    pub sampler_name: String,
    pub sampler_params: PropertyMap,
    pub accelerator_name: String,
    pub accelerator_params: PropertyMap,
    pub integrator_name: String,
    pub integrator_params: PropertyMap,
    pub camera_name: String,
    pub camera_params: PropertyMap,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            filter_name: "box".to_string(),
            filter_params: PropertyMap::new(),
            film_name: "image".to_string(),
            film_params: PropertyMap::new(),
            sampler_name: "halton".to_string(),
            sampler_params: PropertyMap::new(),
            accelerator_name: "bvh".to_string(),
            accelerator_params: PropertyMap::new(),
            integrator_name: "path".to_string(),
            integrator_params: PropertyMap::new(),
            camera_name: "perspective".to_string(),
            camera_params: PropertyMap::new(),
        }
    }
}
