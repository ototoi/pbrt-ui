use pbrt_ui::io::pbrt::export::{SavePbrtOptions, save_pbrt};
use pbrt_ui::io::pbrt::import::load_pbrt;
use std::fs;
use std::path::Path;

#[test]
fn test_pbrt_roundtrip() {
    // Create a simple test PBRT file
    let test_input = r#"# Test PBRT file
LookAt 0 0 10  0 0 0  0 1 0
Camera "perspective" "float fov" [30]
Film "rgb" "integer xresolution" [800] "integer yresolution" [600]
Sampler "halton" "integer pixelsamples" [16]
Integrator "path" "integer maxdepth" [5]

WorldBegin
    AttributeBegin
        Material "matte" "rgb Kd" [0.5 0.5 0.5]
        Shape "sphere" "float radius" [1]
    AttributeEnd
WorldEnd
"#;

    let test_file = "test_input.pbrt";
    let output_file = "test_output.pbrt";

    // Write test input
    fs::write(test_file, test_input).expect("Failed to write test file");

    // Load PBRT
    let node = match load_pbrt(test_file) {
        Ok(n) => n,
        Err(e) => {
            fs::remove_file(test_file).ok();
            panic!("Failed to load PBRT: {}", e);
        }
    };

    // Save PBRT
    let options = SavePbrtOptions::default();
    if let Err(e) = save_pbrt(&node, output_file, &options) {
        fs::remove_file(test_file).ok();
        panic!("Failed to save PBRT: {}", e);
    }

    // Check output exists
    assert!(
        Path::new(output_file).exists(),
        "Output file was not created"
    );

    // Read and print output for inspection
    let output_content = fs::read_to_string(output_file).expect("Failed to read output");
    println!("=== Input ===\n{}", test_input);
    println!("=== Output ===\n{}", output_content);

    // Check basic structure
    assert!(output_content.contains("WorldBegin"), "Missing WorldBegin");
    assert!(output_content.contains("WorldEnd"), "Missing WorldEnd");

    // Cleanup
    fs::remove_file(test_file).ok();
    fs::remove_file(output_file).ok();
}
