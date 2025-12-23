use std::fs;
use std::path::{Path, PathBuf};

use pbrt_ui::preprocessor::Preprocessor;

/// Get the path to the assets/shaders directory
fn get_shaders_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir).join("assets").join("shaders")
}

/// Get the path to the include directory for shader preprocessing
fn get_include_dir() -> PathBuf {
    get_shaders_dir().join("include")
}

/// Preprocess a shader using the preprocessor with the include directory
fn preprocess_shader(source: &str) -> Result<String, String> {
    let include_dir = get_include_dir();
    let mut preprocessor = Preprocessor::with_base_path(&include_dir);

    preprocessor.process(source).map_err(|e| format!("{}", e))
}

/// Find all .wgsl files in a directory, excluding subdirectories
fn find_wgsl_files(dir: &Path) -> Vec<PathBuf> {
    let mut wgsl_files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Some(ext) = path.extension()
                    && ext == "wgsl" {
                        wgsl_files.push(path);
                    }
        }
    }

    wgsl_files.sort();
    wgsl_files
}

#[test]
fn test_all_shaders_compile() {
    let shaders_dir = get_shaders_dir();
    assert!(
        shaders_dir.exists(),
        "Shaders directory does not exist: {}",
        shaders_dir.display()
    );

    let wgsl_files = find_wgsl_files(&shaders_dir);
    assert!(
        !wgsl_files.is_empty(),
        "No .wgsl files found in {}",
        shaders_dir.display()
    );

    let mut failed_shaders = Vec::new();
    let mut success_count = 0;

    for shader_path in &wgsl_files {
        let shader_name = shader_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| shader_path.display().to_string());

        // Read the shader source
        let source = match fs::read_to_string(shader_path) {
            Ok(s) => s,
            Err(e) => {
                failed_shaders.push(format!("{}: Failed to read file: {}", shader_name, e));
                continue;
            }
        };

        // Preprocess the shader
        let preprocessed = match preprocess_shader(&source) {
            Ok(p) => p,
            Err(e) => {
                failed_shaders.push(format!("{}: Preprocessing failed: {}", shader_name, e));
                continue;
            }
        };

        // Validate with naga
        match naga::front::wgsl::parse_str(&preprocessed) {
            Ok(_) => {
                success_count += 1;
            }
            Err(e) => {
                failed_shaders.push(format!("{}: WGSL compilation failed:\n{}", shader_name, e));
            }
        }
    }

    // Report results
    if !failed_shaders.is_empty() {
        let error_msg = format!(
            "\n{} shader(s) failed to compile:\n\n{}",
            failed_shaders.len(),
            failed_shaders.join("\n\n")
        );
        panic!("{}", error_msg);
    }

    println!("✓ All {} shaders compiled successfully", success_count);
}
