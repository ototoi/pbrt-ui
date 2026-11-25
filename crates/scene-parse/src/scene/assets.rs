use super::*;
use std::io::{Seek, SeekFrom};

pub(super) fn read_ply_asset(
    path: &Path,
    max_bytes: usize,
) -> Result<crate::ply::PlyMesh, SceneParseError> {
    let bytes = read_ply_bytes(path, max_bytes)?;
    crate::ply::read_ply(&path.display().to_string(), &bytes)
}

/// Read (and gunzip when needed) a PLY asset within the preview byte budget.
pub(super) fn read_ply_bytes(path: &Path, max_bytes: usize) -> Result<Vec<u8>, SceneParseError> {
    let name = path.display().to_string();
    let size = fs::metadata(path)
        .map_err(|error| SceneParseError::unreadable(&name, error.to_string()))?
        .len();
    if size > max_bytes as u64 {
        return Err(SceneParseError::ResourceExhausted {
            message: format!("PLY asset {name} exceeds the {max_bytes} byte preview asset budget"),
        });
    }
    let mut file = fs::File::open(path)
        .map_err(|error| SceneParseError::unreadable(&name, error.to_string()))?;
    let mut magic = [0; 2];
    let is_gzip = file.read_exact(&mut magic).is_ok() && magic == [0x1f, 0x8b];
    file.seek(SeekFrom::Start(0))
        .map_err(|error| SceneParseError::unreadable(&name, error.to_string()))?;
    let mut bytes = Vec::new();
    if is_gzip {
        flate2::read::MultiGzDecoder::new(file)
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| SceneParseError::unreadable(&name, error.to_string()))?;
    } else {
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| SceneParseError::unreadable(&name, error.to_string()))?;
    }
    if bytes.len() > max_bytes {
        return Err(SceneParseError::ResourceExhausted {
            message: format!(
                "expanded PLY asset {name} exceeds the {max_bytes} byte preview asset budget"
            ),
        });
    }
    Ok(bytes)
}

/// Read raw asset bytes within the preview budget; no decompression.
pub(crate) fn read_asset_bytes(path: &Path, max_bytes: usize) -> Result<Vec<u8>, SceneParseError> {
    let name = path.display().to_string();
    let size = fs::metadata(path)
        .map_err(|error| SceneParseError::unreadable(&name, error.to_string()))?
        .len();
    if size > max_bytes as u64 {
        return Err(SceneParseError::ResourceExhausted {
            message: format!("asset {name} exceeds the {max_bytes} byte preview budget"),
        });
    }
    fs::read(path).map_err(|error| SceneParseError::unreadable(&name, error.to_string()))
}
