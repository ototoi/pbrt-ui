use scene_view::{PreviewMode, ViewerCamera, render_preview};
use std::collections::BTreeMap;
use protocol::geometry::*;
use protocol::view::{Camera, DisplayMode, Projection, ViewState};

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    std::fs::write(&path, "WorldBegin\nLightSource \"point\" \"point from\" [0 0 5] \"rgb I\" [I I I]\nMakeNamedMaterial \"m\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nNamedMaterial \"m\"\nShape \"sphere\"\nWorldEnd\n".replace("I", "1")).unwrap();
    let preview = scene_parse::parse_scene(&path, scene_parse::ParseOptions::default()).unwrap();
    println!("lights: {} materials: {}", preview.manifest.lights.len(), preview.manifest.materials.len());
    for light in &preview.manifest.lights { println!("  light params: {:?}", light.parameters); }
    let camera = ViewerCamera { eye: [0., 0., 4.], target: [0.; 3], up: [0., 1., 0.], fov_y_degrees: 45., near: 0.1, far: 100. };
    for mode in [PreviewMode::Solid, PreviewMode::Lit] {
        let frame = render_preview(&preview.manifest, &preview.blobs, camera, 32, 32, mode).unwrap();
        let mut sum = 0.; let mut count = 0;
        for px in frame.rgba.chunks_exact(4) { if px[3] == 255 { sum += px[0] as f32 + px[1] as f32 + px[2] as f32; count += 1; } }
        println!("{mode:?} luma {} count {}", sum / (3.*count.max(1) as f32), count);
        println!("  center pixel: {:?}", &frame.rgba[16*4*32+16*4..16*4*32+16*4+4]);
    }
    let _ = (Camera::default(), DisplayMode::Solid, ViewState::default(), BTreeMap::<String, Vec<u8>>::new(), Bounds { min: [0.;3], max: [1.;3] }, Projection::Perspective);
}
