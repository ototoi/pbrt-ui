use std::path::PathBuf;

pub fn copy_file(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    if !src.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Source file does not exist: {:?}", src),
        ));
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dst.exists() {
        let src_stat = std::fs::metadata(src)?;
        let dst_stat = std::fs::metadata(dst)?;
        let src_mod = src_stat.modified()?;
        let dst_mod = dst_stat.modified()?;
        if src_mod > dst_mod {
            std::fs::copy(src, dst)?;
        }
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}
