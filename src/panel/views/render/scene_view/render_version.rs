use std::sync::Arc;

use uuid::Uuid;

use eframe::glow;
use glow::HasContext as _;

pub fn get_gl_version(gl: &Arc<glow::Context>) -> String {
    unsafe {
        return gl.get_parameter_string(glow::VERSION);
    }
}

pub fn get_glsl_version(gl: &Arc<glow::Context>) -> String {
    unsafe {
        return gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION);
    }
}

const GLSL_VERSION_LINE: [(&str, &str); 9] = [
    ("1.3", "#version 130"),
    ("1.4", "#version 140"),
    ("1.5", "#version 150"),
    ("3.3", "#version 330"),
    ("4.0", "#version 400"),
    ("4.1", "#version 410"),
    ("4.2", "#version 420"),
    ("4.3", "#version 430"),
    ("4.4", "#version 440"),
];

pub fn get_glsl_version_line(gl: &Arc<glow::Context>) -> Option<String> {
    let version_str = get_glsl_version(gl);
    for (version, line) in GLSL_VERSION_LINE.iter() {
        if version_str.contains(version) {
            return Some(line.to_string());
        }
    }
    return None;
}