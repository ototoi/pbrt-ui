use image::{ImageBuffer, Luma, Rgba};

/// Converts a grayscale heightmap (Luma8) to a normal map (RGBA8).
///
/// The function calculates surface normals from the heightmap by computing
/// the gradient at each pixel. The resulting normal vectors are stored in
/// the RGB channels of the output image, with the alpha channel set to 255.
///
/// # Arguments
/// * `heightmap` - The input grayscale image where pixel intensity represents height
///
/// # Returns
/// An RGBA image where:
/// - R channel: X component of the normal vector (mapped from [-1,1] to [0,255])
/// - G channel: Y component of the normal vector (mapped from [-1,1] to [0,255])
/// - B channel: Z component of the normal vector (mapped from [0,1] to [0,255])
/// - A channel: Always 255 (fully opaque)
pub fn convert_luma8_to_normal_map(
    heightmap: &ImageBuffer<Luma<u8>, Vec<u8>>,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = heightmap.dimensions();
    let mut normal_map = ImageBuffer::new(width, height);

    let strength = 1.0; // Normal map strength factor

    for y in 0..height {
        for x in 0..width {
            // Sample neighboring pixels with wrapping at edges
            // Wrapping ensures seamless tiling for tileable textures
            let left = heightmap.get_pixel(if x > 0 { x - 1 } else { width - 1 }, y)[0] as f32;
            let right = heightmap.get_pixel(if x < width - 1 { x + 1 } else { 0 }, y)[0] as f32;
            let top = heightmap.get_pixel(x, if y > 0 { y - 1 } else { height - 1 })[0] as f32;
            let bottom = heightmap.get_pixel(x, if y < height - 1 { y + 1 } else { 0 })[0] as f32;

            // Calculate gradients (Sobel-like operator)
            let dx = (right - left) / 255.0 * strength;
            let dy = (bottom - top) / 255.0 * strength;

            // Calculate normal vector
            // The cross product of tangent vectors gives us the normal
            let nx = -dx;
            let ny = -dy;
            let nz = 1.0;

            // Normalize the vector
            let length = (nx * nx + ny * ny + nz * nz).sqrt();
            let nx = nx / length;
            let ny = ny / length;
            let nz = nz / length;

            // Map from [-1,1] to [0,255] for all components
            // Standard normal map encoding: (nx, ny, nz) -> (R, G, B)
            let r = ((nx * 0.5 + 0.5) * 255.0) as u8;
            let g = ((ny * 0.5 + 0.5) * 255.0) as u8;
            let b = ((nz * 0.5 + 0.5) * 255.0) as u8;
            let a = 255u8;

            normal_map.put_pixel(x, y, Rgba([r, g, b, a]));
        }
    }

    normal_map
}

pub fn convert_luma32f_to_normal_map(
    heightmap: &ImageBuffer<Luma<f32>, Vec<f32>>,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = heightmap.dimensions();
    let mut normal_map = ImageBuffer::new(width, height);

    let strength = 1.0; // Normal map strength factor

    for y in 0..height {
        for x in 0..width {
            // Sample neighboring pixels with wrapping at edges
            // Wrapping ensures seamless tiling for tileable textures
            let left = heightmap.get_pixel(if x > 0 { x - 1 } else { width - 1 }, y)[0];
            let right = heightmap.get_pixel(if x < width - 1 { x + 1 } else { 0 }, y)[0];
            let top = heightmap.get_pixel(x, if y > 0 { y - 1 } else { height - 1 })[0];
            let bottom = heightmap.get_pixel(x, if y < height - 1 { y + 1 } else { 0 })[0];

            // Calculate gradients (Sobel-like operator)
            let dx = (right - left) * strength;
            let dy = (bottom - top) * strength;

            // Calculate normal vector
            // The cross product of tangent vectors gives us the normal
            let nx = -dx;
            let ny = -dy;
            let nz = 1.0;

            // Normalize the vector
            let length = (nx * nx + ny * ny + nz * nz).sqrt();
            let nx = nx / length;
            let ny = ny / length;
            let nz = nz / length;

            // Map from [-1,1] to [0,255] for all components
            // Standard normal map encoding: (nx, ny, nz) -> (R, G, B)
            let r = ((nx * 0.5 + 0.5) * 255.0) as u8;
            let g = ((ny * 0.5 + 0.5) * 255.0) as u8;
            let b = ((nz * 0.5 + 0.5) * 255.0) as u8;
            let a = 255u8;

            normal_map.put_pixel(x, y, Rgba([r, g, b, a]));
        }
    }

    normal_map
}
