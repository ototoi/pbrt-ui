use rust_embed::Embed;
use std::path::PathBuf;

#[derive(Embed)]
#[folder = "assets/"]
#[exclude = ".DS_Store"]
pub struct Assets;

pub fn copy_assets_to_cache() -> std::io::Result<PathBuf> {
    let cache_dir = dirs::cache_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "Cache directory not found")
    })?;
    let assets_cache_path = cache_dir.join("pbrt_ui").join("assets");
    std::fs::create_dir_all(&assets_cache_path)?;

    for file in Assets::iter() {
        if let Some(embedded_file) = Assets::get(&file) {
            let output_path = assets_cache_path.join(file.to_string());
            let dirname = output_path.parent().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::Other, "Failed to get parent directory")
            })?;
            std::fs::create_dir_all(dirname)?;
            std::fs::write(&output_path, embedded_file.data.as_ref())?;
        }
    }
    Ok(assets_cache_path)
}
