#![allow(non_snake_case)]

use glam::Vec4;
use image::{ImageBuffer, Rgba};
use std::fmt::Write;
use std::io::Write as IoWrite;
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
            expected_len,
            width,
            height,
            data.len()
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

/// Generate Rust code string for a single LTC texture array
///
/// # Arguments
/// * `const_name` - Name of the const array (e.g., "LTC_GGX1")
/// * `data` - Vector of Vec4 containing texture data
/// * `width` - Width of the texture
/// * `height` - Height of the texture
///
/// # Returns
/// * `Result<String, String>` - Generated Rust code or error message
pub fn generate_ltc_array_code(
    const_name: &str,
    data: &[Vec4],
    width: usize,
    height: usize,
) -> Result<String, String> {
    let expected_len = width * height;
    if data.len() != expected_len {
        return Err(format!(
            "Data length mismatch: expected {} elements ({}x{}), got {}",
            expected_len,
            width,
            height,
            data.len()
        ));
    }

    // Pre-allocate string capacity for better performance
    let total_floats = expected_len * 4;
    let estimated_size = total_floats * 15 + 100; // ~15 chars per float plus overhead
    let mut code = String::with_capacity(estimated_size);
    code.push_str("#[rustfmt::skip]\n");
    code.push_str(&format!(
        "pub const {}: [f32; {}] = [\n",
        const_name, total_floats
    ));

    for j in 0..height {
        for i in 0..width {
            let idx = i + j * width;
            // Writing to a String cannot fail, so unwrap is safe
            let v = data[idx];
            let _ = write!(
                &mut code,
                "    {:>9.6}, {:>9.6}, {:>9.6}, {:>9.6}, // [{:>2}, {:>2}] \n",
                v.x, v.y, v.z, v.w, i, j
            );
        }
    }

    code.push_str("];\n");
    Ok(code)
}

/// Write multiple LTC arrays to a single Rust source file
///
/// # Arguments
/// * `path` - Output file path for the Rust source file
/// * `brdf_name` - Name of the BRDF (e.g., "ggx", "beckmann")
/// * `tex1` - First texture data (inverse matrix parameters)
/// * `tex2` - Second texture data (magnitude, fresnel, sphere)
/// * `width` - Width of the textures
/// * `height` - Height of the textures
///
/// # Returns
/// * `Result<(), String>` - Ok if successful, Err with error message otherwise
pub fn write_multiple_ltc_arrays<P: AsRef<Path>>(
    path: P,
    brdf_name: &str,
    tex1: &[Vec4],
    tex2: &[Vec4],
    width: usize,
    height: usize,
) -> Result<(), String> {
    // Convert brdf_name to uppercase for const names
    let brdf_upper = brdf_name.to_uppercase();
    let const_name1 = format!("LTC_{}_1", brdf_upper);
    let const_name2 = format!("LTC_{}_2", brdf_upper);

    // Generate code for both arrays
    let code1 = generate_ltc_array_code(&const_name1, tex1, width, height)?;
    let code2 = generate_ltc_array_code(&const_name2, tex2, width, height)?;

    // Combine into a single file with header comment
    let mut file_content = String::new();
    file_content.push_str("// Auto-generated LTC texture data\n");
    file_content.push_str(&format!("// BRDF: {}\n", brdf_name));
    file_content.push_str(&format!("// Size: {}x{}\n\n", width, height));
    file_content.push_str(&code1);
    file_content.push('\n');
    file_content.push_str(&code2);

    // Write to file
    let mut file = std::fs::File::create(path.as_ref())
        .map_err(|e| format!("Failed to create file: {}", e))?;
    file.write_all(file_content.as_bytes())
        .map_err(|e| format!("Failed to write to file: {}", e))?;

    Ok(())
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

    #[test]
    fn test_generate_ltc_array_code_validates_length() {
        let wrong_data = vec![Vec4::ZERO; 10];
        let width = 64;
        let height = 64;

        let result = generate_ltc_array_code("TEST", &wrong_data, width, height);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Data length mismatch"));
    }

    #[test]
    fn test_generate_ltc_array_code_format() {
        let width = 2;
        let height = 2;
        let data = vec![
            Vec4::new(1.0, 2.0, 3.0, 4.0),
            Vec4::new(5.0, 6.0, 7.0, 8.0),
            Vec4::new(9.0, 10.0, 11.0, 12.0),
            Vec4::new(13.0, 14.0, 15.0, 16.0),
        ];

        let result = generate_ltc_array_code("LTC_TEST", &data, width, height);
        assert!(result.is_ok());

        let code = result.unwrap();
        assert!(code.contains("pub const LTC_TEST: [f32; 16] = ["));
        assert!(code.contains(" 1.000000,  2.000000,  3.000000,  4.000000,"));
        assert!(code.contains(" 5.000000,  6.000000,  7.000000,  8.000000,"));
        assert!(code.contains(" 9.000000, 10.000000, 11.000000, 12.000000,"));
        assert!(code.contains("13.000000, 14.000000, 15.000000, 16.000000,"));
    }

    #[test]
    fn test_write_multiple_ltc_arrays_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("ltc_arrays.rs");

        let width = 2;
        let height = 2;
        let tex1 = vec![Vec4::new(1.0, 2.0, 3.0, 4.0); 4];
        let tex2 = vec![Vec4::new(5.0, 6.0, 7.0, 8.0); 4];

        let result = write_multiple_ltc_arrays(&path, "ggx", &tex1, &tex2, width, height);
        assert!(result.is_ok());
        assert!(path.exists());

        // Read and verify content
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("// Auto-generated LTC texture data"));
        assert!(content.contains("// BRDF: ggx"));
        assert!(content.contains("pub const LTC_GGX_1"));
        assert!(content.contains("pub const LTC_GGX_2"));
    }

    #[test]
    fn test_write_multiple_ltc_arrays_uppercase_conversion() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("ltc_arrays.rs");

        let width = 2;
        let height = 2;
        let tex1 = vec![Vec4::ZERO; 4];
        let tex2 = vec![Vec4::ZERO; 4];

        let result = write_multiple_ltc_arrays(&path, "beckmann", &tex1, &tex2, width, height);
        assert!(result.is_ok());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("LTC_BECKMANN_1"));
        assert!(content.contains("LTC_BECKMANN_2"));
    }
}
