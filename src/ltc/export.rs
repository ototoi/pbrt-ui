#![allow(non_snake_case)]

use super::parameters::N;
use glam::Vec4;
use image::{ImageBuffer, Rgba};
use std::path::Path;

/// Write LTC texture data to EXR file
/// 
/// # Arguments
/// * `path` - Output file path for the EXR file
/// * `data` - Vector of Vec4 containing texture data (N*N elements)
/// 
/// # Returns
/// * `Result<(), String>` - Ok if successful, Err with error message otherwise
pub fn write_exr<P: AsRef<Path>>(path: P, data: &[Vec4]) -> Result<(), String> {
    if data.len() != N * N {
        return Err(format!(
            "Data length mismatch: expected {} elements, got {}",
            N * N,
            data.len()
        ));
    }

    let img: ImageBuffer<Rgba<f32>, Vec<f32>> = ImageBuffer::from_fn(N as u32, N as u32, |x, y| {
        let idx = (x as usize) + (y as usize) * N;
        let v = data[idx];
        Rgba([v.x, v.y, v.z, v.w])
    });

    img.save(path.as_ref())
        .map_err(|e| format!("Failed to save EXR file: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_write_exr_validates_data_length() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.exr");
        
        // Create data with wrong length
        let wrong_data = vec![Vec4::ZERO; 10];
        
        let result = write_exr(&path, &wrong_data);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Data length mismatch"));
    }

    #[test]
    fn test_write_exr_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.exr");
        
        // Create valid data
        let data = vec![Vec4::new(1.0, 0.5, 0.25, 0.75); N * N];
        
        let result = write_exr(&path, &data);
        assert!(result.is_ok());
        assert!(path.exists());
        
        // Verify file is not empty
        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }
}
