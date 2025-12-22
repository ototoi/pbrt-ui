// Test for FBM texture generation
use pbrt_ui::conversion::texture_node::{render_texture_image, TextureSizeType, DynaImage};
use pbrt_ui::model::base::{Matrix4x4, Property, PropertyMap};
use pbrt_ui::model::scene::Texture;
use std::collections::HashMap;

#[test]
fn test_fbm_texture_basic() {
    // Create a basic FBM texture with default parameters
    let props = PropertyMap::new();
    
    // Create FBM texture
    let texture = Texture::new("test_fbm", "float", "fbm", None, &props, &Matrix4x4::default());
    
    let dependencies = HashMap::new();
    
    // Try to render at icon size
    let result = render_texture_image(&texture, &dependencies, TextureSizeType::Icon);
    
    assert!(result.is_some(), "FBM texture should render successfully");
    
    if let Some(DynaImage::ImageLuma32F(img)) = result {
        assert_eq!(img.width(), 64, "Icon size should be 64x64");
        assert_eq!(img.height(), 64, "Icon size should be 64x64");
        
        // Check that values are in valid range [0, 1]
        for pixel in img.pixels() {
            let value = pixel[0];
            assert!(value >= 0.0 && value <= 1.0, "Pixel values should be in [0, 1] range, got {}", value);
        }
        
        // Check that we have some variation (not all the same value)
        let first_value = img.get_pixel(0, 0)[0];
        let has_variation = img.pixels().any(|p| (p[0] - first_value).abs() > 0.01);
        assert!(has_variation, "FBM texture should have variation, not be constant");
    } else {
        panic!("FBM texture should be ImageLuma32F type");
    }
}

#[test]
fn test_fbm_texture_with_custom_params() {
    // Create FBM texture with custom parameters
    let mut props = PropertyMap::new();
    props.insert("integer octaves", Property::from(4));
    props.insert("float roughness", Property::from(0.7));
    
    let texture = Texture::new("test_fbm_custom", "float", "fbm", None, &props, &Matrix4x4::default());
    
    let dependencies = HashMap::new();
    
    // Try to render at display size
    let result = render_texture_image(&texture, &dependencies, TextureSizeType::Display);
    
    assert!(result.is_some(), "FBM texture with custom params should render successfully");
    
    if let Some(DynaImage::ImageLuma32F(img)) = result {
        assert_eq!(img.width(), 256, "Display size should be 256x256");
        assert_eq!(img.height(), 256, "Display size should be 256x256");
        
        // Check that values are in valid range [0, 1]
        for pixel in img.pixels() {
            let value = pixel[0];
            assert!(value >= 0.0 && value <= 1.0, "Pixel values should be in [0, 1] range");
        }
    } else {
        panic!("FBM texture should be ImageLuma32F type");
    }
}

#[test]
fn test_fbm_texture_render_size() {
    // Create FBM texture
    let props = PropertyMap::new();
    let texture = Texture::new("test_fbm_render", "float", "fbm", None, &props, &Matrix4x4::default());
    
    let dependencies = HashMap::new();
    
    // Try to render at render size (1024x1024)
    let result = render_texture_image(&texture, &dependencies, TextureSizeType::Render);
    
    assert!(result.is_some(), "FBM texture at render size should render successfully");
    
    if let Some(DynaImage::ImageLuma32F(img)) = result {
        assert_eq!(img.width(), 1024, "Render size should be 1024x1024");
        assert_eq!(img.height(), 1024, "Render size should be 1024x1024");
    } else {
        panic!("FBM texture should be ImageLuma32F type");
    }
}
