// Manual visual test for FBM texture - generates sample images
use pbrt_ui::conversion::texture_node::{DynaImage, TextureSizeType, render_texture_image};
use pbrt_ui::model::base::{Matrix4x4, Property, PropertyMap};
use pbrt_ui::model::scene::Texture;
use std::collections::HashMap;

fn main() {
    println!("Generating FBM texture samples...");

    // Create FBM texture with default parameters
    let props = PropertyMap::new();
    let texture = Texture::new(
        "fbm_default",
        "float",
        "fbm",
        None,
        &props,
        &Matrix4x4::default(),
    );
    let dependencies = HashMap::new();

    if let Some(DynaImage::ImageLuma32F(img)) =
        render_texture_image(&texture, &dependencies, TextureSizeType::Display)
    {
        // Convert to 8-bit for saving
        let img_u8: image::ImageBuffer<image::Luma<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
                let value = img.get_pixel(x, y)[0];
                let byte_value = (value * 255.0).clamp(0.0, 255.0) as u8;
                image::Luma([byte_value])
            });
        img_u8
            .save("/tmp/fbm_default.png")
            .expect("Failed to save default FBM texture");
        println!("Saved: /tmp/fbm_default.png (octaves=8, roughness=0.5)");
    }

    // Create FBM texture with high octaves
    let mut props_high = PropertyMap::new();
    props_high.insert("integer octaves", Property::from(12));
    props_high.insert("float roughness", Property::from(0.5));
    let texture_high = Texture::new(
        "fbm_high_octaves",
        "float",
        "fbm",
        None,
        &props_high,
        &Matrix4x4::default(),
    );

    if let Some(DynaImage::ImageLuma32F(img)) =
        render_texture_image(&texture_high, &dependencies, TextureSizeType::Display)
    {
        let img_u8: image::ImageBuffer<image::Luma<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
                let value = img.get_pixel(x, y)[0];
                let byte_value = (value * 255.0).clamp(0.0, 255.0) as u8;
                image::Luma([byte_value])
            });
        img_u8
            .save("/tmp/fbm_high_octaves.png")
            .expect("Failed to save high octaves FBM texture");
        println!("Saved: /tmp/fbm_high_octaves.png (octaves=12, roughness=0.5)");
    }

    // Create FBM texture with low roughness (smoother)
    let mut props_smooth = PropertyMap::new();
    props_smooth.insert("integer octaves", Property::from(8));
    props_smooth.insert("float roughness", Property::from(0.3));
    let texture_smooth = Texture::new(
        "fbm_smooth",
        "float",
        "fbm",
        None,
        &props_smooth,
        &Matrix4x4::default(),
    );

    if let Some(DynaImage::ImageLuma32F(img)) =
        render_texture_image(&texture_smooth, &dependencies, TextureSizeType::Display)
    {
        let img_u8: image::ImageBuffer<image::Luma<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
                let value = img.get_pixel(x, y)[0];
                let byte_value = (value * 255.0).clamp(0.0, 255.0) as u8;
                image::Luma([byte_value])
            });
        img_u8
            .save("/tmp/fbm_smooth.png")
            .expect("Failed to save smooth FBM texture");
        println!("Saved: /tmp/fbm_smooth.png (octaves=8, roughness=0.3)");
    }

    // Create FBM texture with high roughness (more variation)
    let mut props_rough = PropertyMap::new();
    props_rough.insert("integer octaves", Property::from(8));
    props_rough.insert("float roughness", Property::from(0.7));
    let texture_rough = Texture::new(
        "fbm_rough",
        "float",
        "fbm",
        None,
        &props_rough,
        &Matrix4x4::default(),
    );

    if let Some(DynaImage::ImageLuma32F(img)) =
        render_texture_image(&texture_rough, &dependencies, TextureSizeType::Display)
    {
        let img_u8: image::ImageBuffer<image::Luma<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
                let value = img.get_pixel(x, y)[0];
                let byte_value = (value * 255.0).clamp(0.0, 255.0) as u8;
                image::Luma([byte_value])
            });
        img_u8
            .save("/tmp/fbm_rough.png")
            .expect("Failed to save rough FBM texture");
        println!("Saved: /tmp/fbm_rough.png (octaves=8, roughness=0.7)");
    }

    println!("\nAll FBM texture samples generated successfully!");
    println!("Check /tmp/ directory for the generated PNG files.");
}
