use crate::io::pbrt::ParseTarget;
use crate::model::base::PropertyMap as ParamSet;
use crate::model::base::Property;
use std::io::Write;

pub type Float = f32;

pub struct SaveTarget<W: Write> {
    pub(super) writer: W,
    indent_level: usize,
    pretty_print: bool,
}

impl<W: Write> SaveTarget<W> {
    pub fn new(writer: W, pretty_print: bool) -> Self {
        Self {
            writer,
            indent_level: 0,
            pretty_print,
        }
    }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.pretty_print {
            let indent = "    ".repeat(self.indent_level);
            write!(self.writer, "{}{}\n", indent, line)
        } else {
            write!(self.writer, "{}\n", line)
        }
    }

    fn write_command(&mut self, cmd: &str, args: &str) -> std::io::Result<()> {
        if args.is_empty() {
            self.write_line(cmd)
        } else {
            self.write_line(&format!("{} {}", cmd, args))
        }
    }

    fn format_params(&self, params: &ParamSet) -> String {
        let mut result = String::new();
        for (param_type, key, value) in params.0.iter() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&self.format_property(param_type, key, value));
        }
        result
    }

    fn format_property(&self, param_type: &str, key: &str, value: &Property) -> String {
        match value {
            Property::Floats(v) => {
                let values = v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
                format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
            }
            Property::Ints(v) => {
                let values = v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
                format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
            }
            Property::Strings(v) => {
                if v.len() == 1 {
                    format!("\"{}\" \"{}\"", Self::make_param_key(param_type, key), v[0])
                } else {
                    let values = v.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(" ");
                    format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
                }
            }
            Property::Bools(v) => {
                if v.len() == 1 {
                    let value_str = if v[0] { "true" } else { "false" };
                    format!("\"{}\" \"{}\"", Self::make_param_key(param_type, key), value_str)
                } else {
                    // Multiple bools as array
                    let values = v.iter().map(|b| if *b { "true" } else { "false" }).collect::<Vec<_>>().join(" ");
                    format!("\"{}\" [{}]", Self::make_param_key(param_type, key), values)
                }
            }
        }
    }

    fn make_param_key(param_type: &str, key: &str) -> String {
        if param_type.is_empty() {
            key.to_string()
        } else {
            format!("{} {}", param_type, key)
        }
    }

    pub fn finish(mut self) -> std::io::Result<W> {
        self.writer.flush()?;
        Ok(self.writer)
    }
}

impl<W: Write> ParseTarget for SaveTarget<W> {
    fn cleanup(&mut self) {
        // No-op for save target
    }

    fn identity(&mut self) {
        let _ = self.write_line("Identity");
    }

    fn translate(&mut self, dx: Float, dy: Float, dz: Float) {
        let _ = self.write_command("Translate", &format!("{} {} {}", dx, dy, dz));
    }

    fn rotate(&mut self, angle: Float, ax: Float, ay: Float, az: Float) {
        let _ = self.write_command("Rotate", &format!("{} {} {} {}", angle, ax, ay, az));
    }

    fn scale(&mut self, sx: Float, sy: Float, sz: Float) {
        let _ = self.write_command("Scale", &format!("{} {} {}", sx, sy, sz));
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
        let _ = self.write_command(
            "LookAt",
            &format!("{} {} {} {} {} {} {} {} {}", ex, ey, ez, lx, ly, lz, ux, uy, uz),
        );
    }

    fn concat_transform(&mut self, transform: &[Float]) {
        if transform.len() == 16 {
            let values = transform.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
            let _ = self.write_command("ConcatTransform", &format!("[{}]", values));
        }
    }

    fn transform(&mut self, transform: &[Float]) {
        if transform.len() == 16 {
            let values = transform.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
            let _ = self.write_command("Transform", &format!("[{}]", values));
        }
    }

    fn coordinate_system(&mut self, name: &str) {
        let _ = self.write_command("CoordinateSystem", &format!("\"{}\"", name));
    }

    fn coord_sys_transform(&mut self, name: &str) {
        let _ = self.write_command("CoordSysTransform", &format!("\"{}\"", name));
    }

    fn active_transform_all(&mut self) {
        let _ = self.write_line("ActiveTransform All");
    }

    fn active_transform_end_time(&mut self) {
        let _ = self.write_line("ActiveTransform EndTime");
    }

    fn active_transform_start_time(&mut self) {
        let _ = self.write_line("ActiveTransform StartTime");
    }

    fn transform_times(&mut self, start: Float, end: Float) {
        let _ = self.write_command("TransformTimes", &format!("{} {}", start, end));
    }

    fn pixel_filter(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("PixelFilter", &format!("\"{}\" {}", name, params_str));
    }

    fn film(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Film", &format!("\"{}\" {}", name, params_str));
    }

    fn sampler(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Sampler", &format!("\"{}\" {}", name, params_str));
    }

    fn accelerator(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Accelerator", &format!("\"{}\" {}", name, params_str));
    }

    fn integrator(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Integrator", &format!("\"{}\" {}", name, params_str));
    }

    fn camera(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Camera", &format!("\"{}\" {}", name, params_str));
    }

    fn make_named_medium(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("MakeNamedMedium", &format!("\"{}\" {}", name, params_str));
    }

    fn medium_interface(&mut self, inside_name: &str, outside_name: &str) {
        let _ = self.write_command("MediumInterface", &format!("\"{}\" \"{}\"", inside_name, outside_name));
    }

    fn world_begin(&mut self) {
        let _ = self.write_line("WorldBegin");
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn attribute_begin(&mut self) {
        let _ = self.write_line("AttributeBegin");
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn attribute_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("AttributeEnd");
    }

    fn transform_begin(&mut self) {
        let _ = self.write_line("TransformBegin");
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn transform_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("TransformEnd");
    }

    fn texture(&mut self, name: &str, _type: &str, tex_name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Texture", &format!("\"{}\" \"{}\" \"{}\" {}", name, _type, tex_name, params_str));
    }

    fn material(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Material", &format!("\"{}\" {}", name, params_str));
    }

    fn make_named_material(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("MakeNamedMaterial", &format!("\"{}\" {}", name, params_str));
    }

    fn named_material(&mut self, name: &str) {
        let _ = self.write_command("NamedMaterial", &format!("\"{}\"", name));
    }

    fn light_source(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("LightSource", &format!("\"{}\" {}", name, params_str));
    }

    fn area_light_source(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("AreaLightSource", &format!("\"{}\" {}", name, params_str));
    }

    fn shape(&mut self, name: &str, params: &ParamSet) {
        let params_str = self.format_params(params);
        let _ = self.write_command("Shape", &format!("\"{}\" {}", name, params_str));
    }

    fn reverse_orientation(&mut self) {
        let _ = self.write_line("ReverseOrientation");
    }

    fn object_begin(&mut self, name: &str) {
        let _ = self.write_command("ObjectBegin", &format!("\"{}\"", name));
        if self.pretty_print {
            self.indent_level += 1;
        }
    }

    fn object_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("ObjectEnd");
    }

    fn object_instance(&mut self, name: &str) {
        let _ = self.write_command("ObjectInstance", &format!("\"{}\"", name));
    }

    fn world_end(&mut self) {
        if self.pretty_print && self.indent_level > 0 {
            self.indent_level -= 1;
        }
        let _ = self.write_line("WorldEnd");
    }

    fn parse_file(&mut self, filename: &str) {
        // For save target, we might want to write an Include directive
        let _ = self.write_command("Include", &format!("\"{}\"", filename));
    }

    fn parse_string(&mut self, _s: &str) {
        // No-op for save target
    }
}
