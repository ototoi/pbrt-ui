use include_dir::{Dir, include_dir};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// Statically include the shader files at build time
static SHADERS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/render/wgpu/shaders");

/// Copies embedded shader files to the user's cache directory.
///
/// This function extracts shader files that were embedded at build time
/// into a cache directory at `~/.cache/pbrt_ui/shaders` (on Linux/macOS)
/// or the appropriate cache location on other platforms.
///
/// # Returns
/// Returns `Ok(PathBuf)` with the path to the shaders cache directory on success.
///
/// # Errors
/// Returns an `io::Error` if:
/// - The cache directory cannot be determined
/// - Directory creation fails
/// - File writing fails
pub fn copy_shaders_to_cache() -> io::Result<PathBuf> {
    // Get the user's cache directory
    let cache_dir = dirs::cache_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Cache directory not found"))?;

    // Create the pbrt_ui/shaders subdirectory path
    let shaders_cache_path = cache_dir.join("pbrt_ui").join("shaders");

    // Ensure the directory hierarchy exists
    fs::create_dir_all(&shaders_cache_path)?;

    // Recursively copy all shader files from the embedded directory
    copy_dir_recursive(&SHADERS_DIR, &shaders_cache_path)?;

    Ok(shaders_cache_path)
}

/// Recursively copies files from an embedded directory to a filesystem path.
///
/// # Arguments
/// * `embedded_dir` - The embedded directory to copy from
/// * `dest_path` - The destination path on the filesystem
///
/// # Errors
/// Returns an `io::Error` if directory creation or file writing fails
fn copy_dir_recursive(embedded_dir: &Dir, dest_path: &Path) -> io::Result<()> {
    // Copy all files in the current directory
    for file in embedded_dir.files() {
        // Get just the file name, not the full path
        let file_name = file
            .path()
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid file name"))?;
        let file_path = dest_path.join(file_name);

        // Write the file contents
        fs::write(&file_path, file.contents())?;
    }

    // Recursively process subdirectories
    for subdir in embedded_dir.dirs() {
        let subdir_name = subdir
            .path()
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid directory name"))?;
        let subdir_path = dest_path.join(subdir_name);

        // Create the subdirectory
        fs::create_dir_all(&subdir_path)?;

        // Recursively copy the subdirectory
        copy_dir_recursive(subdir, &subdir_path)?;
    }

    Ok(())
}
