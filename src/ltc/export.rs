#![allow(non_snake_case)]

use glam::Vec4;
use image::{ImageBuffer, Rgba};
use std::path::Path;

/// Write LTC texture data to EXR file
/// 
/// # Arguments
/// * `path` - Output file path for the EXR file
/// * `data` - Vector of Vec4 containing texture data (width*height elements)
/// * `width` - Width of the texture
/// * `height` - Height of the texture
/// 
/// # Returns
/// * `Result<(), String>` - Ok if successful, Err with error message otherwise
pub fn write_exr<P: AsRef<Path>>(
    path: P,
    data: &[Vec4],
    width: usize,
    height: usize,
) -> Result<(), String> {
    let expected_len = width * height;
    if data.len() != expected_len {
        return Err(format!(
            "Data length mismatch: expected {} elements ({}x{}), got {}",
            expected_len, width, height, data.len()
        ));
    }

    let img: ImageBuffer<Rgba<f32>, Vec<f32>> =
        ImageBuffer::from_fn(width as u32, height as u32, |x, y| {
            // Index calculation: row-major order with bounds guaranteed by data.len() == width*height validation
            // x and y range from 0 to width-1 and height-1, so max index is (width-1) + (height-1)*width = width*height - 1
            let idx = (x as usize) + (y as usize) * width;
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
        let width = 64;
        let height = 64;
        
        let result = write_exr(&path, &wrong_data, width, height);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Data length mismatch"));
    }

    #[test]
    fn test_write_exr_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.exr");
        
        // Create valid data
        let width = 64;
        let height = 64;
        let data = vec![Vec4::new(1.0, 0.5, 0.25, 0.75); width * height];
        
        let result = write_exr(&path, &data, width, height);
        assert!(result.is_ok());
        assert!(path.exists());
        
        // Verify file is not empty
        let metadata = fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }
}
