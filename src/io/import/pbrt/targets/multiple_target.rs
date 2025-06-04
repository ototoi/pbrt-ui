use super::super::parse::ParseTarget;
use crate::models::base::ParamSet;

use std::sync::Arc;
use std::sync::RwLock;

type Float = f32;

#[derive(Default)]
pub struct MultipleTarget {
    pub targets: Vec<Arc<RwLock<dyn ParseTarget>>>,
}

impl MultipleTarget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_target<T: ParseTarget + 'static>(&mut self, target: Arc<RwLock<T>>) {
        self.targets.push(target);
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }
}

impl ParseTarget for MultipleTarget {
    fn cleanup(&mut self) {
        for target in &self.targets {
            target.write().unwrap().cleanup();
        }
    }
    fn identity(&mut self) {
        for target in &self.targets {
            target.write().unwrap().identity();
        }
    }
    fn translate(&mut self, dx: Float, dy: Float, dz: Float) {
        for target in &self.targets {
            target.write().unwrap().translate(dx, dy, dz);
        }
    }
    fn rotate(&mut self, angle: Float, ax: Float, ay: Float, az: Float) {
        for target in &self.targets {
            target.write().unwrap().rotate(angle, ax, ay, az);
        }
    }
    fn scale(&mut self, sx: Float, sy: Float, sz: Float) {
        for target in &self.targets {
            target.write().unwrap().scale(sx, sy, sz);
        }
    }
    fn look_at(
        &mut self,
        ex: Float,
        ey: Float,
        ez: Float,
        lx: Float,
        ly: Float,
        lz: Float,
        ux: Float,
        uy: Float,
        uz: Float,
    ) {
        for target in &self.targets {
            target
                .write()
                .unwrap()
                .look_at(ex, ey, ez, lx, ly, lz, ux, uy, uz);
        }
    }
    fn concat_transform(&mut self, tansform: &[Float]) {
        for target in &self.targets {
            target.write().unwrap().concat_transform(tansform);
        }
    }
    fn transform(&mut self, tansform: &[Float]) {
        for target in &self.targets {
            target.write().unwrap().transform(tansform);
        }
    }
    fn coordinate_system(&mut self, name: &str) {
        for target in &self.targets {
            target.write().unwrap().coordinate_system(name);
        }
    }
    fn coord_sys_transform(&mut self, name: &str) {
        for target in &self.targets {
            target.write().unwrap().coord_sys_transform(name);
        }
    }
    fn active_transform_all(&mut self) {
        for target in &self.targets {
            target.write().unwrap().active_transform_all();
        }
    }
    fn active_transform_end_time(&mut self) {
        for target in &self.targets {
            target.write().unwrap().active_transform_end_time();
        }
    }
    fn active_transform_start_time(&mut self) {
        for target in &self.targets {
            target.write().unwrap().active_transform_start_time();
        }
    }
    fn transform_times(&mut self, start: Float, end: Float) {
        for target in &self.targets {
            target.write().unwrap().transform_times(start, end);
        }
    }
    fn pixel_filter(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().pixel_filter(name, params);
        }
    }
    fn film(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().film(name, params);
        }
    }
    fn sampler(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().sampler(name, params);
        }
    }
    fn accelerator(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().accelerator(name, params);
        }
    }
    fn integrator(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().integrator(name, params);
        }
    }
    fn camera(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().camera(name, params);
        }
    }
    fn make_named_medium(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().make_named_medium(name, params);
        }
    }
    fn medium_interface(&mut self, inside_name: &str, outside_name: &str) {
        for target in &self.targets {
            target
                .write()
                .unwrap()
                .medium_interface(inside_name, outside_name);
        }
    }

    fn world_begin(&mut self) {
        for target in &self.targets {
            target.write().unwrap().world_begin();
        }
    }
    fn attribute_begin(&mut self) {
        for target in &self.targets {
            target.write().unwrap().attribute_begin();
        }
    }
    fn attribute_end(&mut self) {
        for target in &self.targets {
            target.write().unwrap().attribute_end();
        }
    }
    fn transform_begin(&mut self) {
        for target in &self.targets {
            target.write().unwrap().transform_begin();
        }
    }
    fn transform_end(&mut self) {
        for target in &self.targets {
            target.write().unwrap().transform_end();
        }
    }
    fn texture(&mut self, name: &str, _type: &str, tex_name: &str, params: &ParamSet) {
        for target in &self.targets {
            target
                .write()
                .unwrap()
                .texture(name, _type, tex_name, params);
        }
    }
    fn material(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().material(name, params);
        }
    }
    fn make_named_material(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().make_named_material(name, params);
        }
    }
    fn named_material(&mut self, name: &str) {
        for target in &self.targets {
            target.write().unwrap().named_material(name);
        }
    }
    fn light_source(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().light_source(name, params);
        }
    }
    fn area_light_source(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().area_light_source(name, params);
        }
    }
    fn shape(&mut self, name: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().shape(name, params);
        }
    }
    fn reverse_orientation(&mut self) {
        for target in &self.targets {
            target.write().unwrap().reverse_orientation();
        }
    }
    fn object_begin(&mut self, name: &str) {
        for target in &self.targets {
            target.write().unwrap().object_begin(name);
        }
    }
    fn object_end(&mut self) {
        for target in &self.targets {
            target.write().unwrap().object_end();
        }
    }
    fn object_instance(&mut self, name: &str) {
        for target in &self.targets {
            target.write().unwrap().object_instance(name);
        }
    }
    fn world_end(&mut self) {
        for target in &self.targets {
            target.write().unwrap().world_end();
        }
    }

    fn parse_file(&mut self, filename: &str) {
        for target in &self.targets {
            target.write().unwrap().parse_file(filename);
        }
    }
    fn parse_string(&mut self, s: &str) {
        for target in &self.targets {
            target.write().unwrap().parse_string(s);
        }
    }

    //----------------------------------------
    fn work_dir_begin(&mut self, path: &str) {
        for target in &self.targets {
            target.write().unwrap().work_dir_begin(path);
        }
    }
    fn work_dir_end(&mut self) {
        for target in &self.targets {
            target.write().unwrap().work_dir_end();
        }
    }
    fn include(&mut self, filename: &str, params: &ParamSet) {
        for target in &self.targets {
            target.write().unwrap().include(filename, params);
        }
    }
    //----------------------------------------
}
