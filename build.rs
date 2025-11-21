use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() {
    // Get the output directory where build artifacts are stored
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR environment variable not set");
    let out_path = Path::new(&out_dir);

    // Define source and destination paths
    let shader_src = Path::new("src/render/wgpu/shaders");
    let shader_dst = out_path.join("shaders");

    // Copy the shaders directory recursively
    if let Err(e) = copy_dir_recursive(&shader_src, &shader_dst) {
        panic!(
            "Failed to copy shaders from {:?} to {:?}: {}",
            shader_src, shader_dst, e
        );
    }

    // Watch individual files to ensure proper incremental rebuilds
    if let Err(e) = watch_dir_files(&shader_src) {
        panic!(
            "Failed to set up file watching for {:?}: {}",
            shader_src, e
        );
    }
}

/// Recursively copy a directory and its contents
fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    // Create the destination directory if it doesn't exist
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    // Iterate over entries in the source directory
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);

        if src_path.is_dir() {
            // Recursively copy subdirectories
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            // Copy files
            fs::copy(&src_path, &dst_path).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("Failed to copy {:?} to {:?}: {}", src_path, dst_path, e),
                )
            })?;
        }
    }

    Ok(())
}

/// Recursively watch all files in a directory for changes
fn watch_dir_files(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Recursively watch subdirectories
            watch_dir_files(&path)?;
        } else if path.is_file() {
            // Tell cargo to rerun if this specific file changes
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    Ok(())
}
