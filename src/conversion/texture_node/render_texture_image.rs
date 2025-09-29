use super::texture_node::TexturePurpose;
use crate::conversion::spectrum::Spectrum;
use crate::model::base::Property;
use crate::model::scene::Texture;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use crate::conversion::texture_node::DynaImage;
use image::buffer::ConvertBuffer as _;
use image::GenericImageView;
use image::ImageBuffer;

const ICON_SIZE: u32 = 64;
const DISPLAY_SIZE: u32 = 256;
const RENDER_SIZE: u32 = 1024;

/*
fn is_float_image(image: &DynaImage) -> bool {
    match image {
        DynaImage::ImageRgb32F(_) => true,
        DynaImage::ImageRgba32F(_) => true,
        _ => false,
    }
}
*/

fn get_color_texture_image(texture: &Texture, key: &str) -> Option<DynaImage> {
    let props = texture.as_property_map();
    if let Some((key_type, key_name, value)) = props.entry(key) {
        if let Property::Floats(v) = value {
            if key_type == "blackbody" {
                let s = Spectrum::from_blackbody(&v);
                let color = s.to_rgb();
                let color = image::Rgb([color[0] as f32, color[1] as f32, color[2] as f32]);
                let image_buffer = image::ImageBuffer::from_pixel(1, 1, color);
                return Some(DynaImage::ImageRgb32F(image_buffer));
            } else {
                if v.len() == 1 {
                    let value = image::Luma([v[0] as f32]);
                    let image_buffer = image::ImageBuffer::from_pixel(1, 1, value);
                    return Some(DynaImage::ImageLuma32F(image_buffer));
                } else if v.len() == 3 {
                    let color = image::Rgb([(v[0]) as f32, (v[1]) as f32, (v[2]) as f32]);
                    let image_buffer = image::ImageBuffer::from_pixel(1, 1, color);
                    return Some(DynaImage::ImageRgb32F(image_buffer));
                }
            }
        } else if let Property::Strings(_name) = value {
            if key_type == "spectrum" {
                let fullpath_name = format!("{}_fullpath", key_name);
                if let Some(src) = props.get(&fullpath_name) {
                    if let Property::Strings(v) = src {
                        assert!(
                            v.len() == 1,
                            "Spectrum fullpath must have exactly one value"
                        );
                        let fullpath = v[0].clone();
                        if let Ok(s) = Spectrum::load_from_file(&fullpath) {
                            let color = s.to_rgb();
                            let color =
                                image::Rgb([color[0] as f32, color[1] as f32, color[2] as f32]);
                            let image_buffer = image::ImageBuffer::from_pixel(1, 1, color);
                            return Some(DynaImage::ImageRgb32F(image_buffer));
                        }
                    }
                }
            }
        }
    }
    return None;
}

fn srgb_to_linear(value: u8) -> f32 {
    let v = (value as f32) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}


fn convert_to_linear_float_image(image: &DynaImage) -> DynaImage {
    match image {
        DynaImage::ImageLuma8(img) => {
            let float_image: ImageBuffer<image::Luma<f32>, Vec<f32>> = img.clone().convert();
            return DynaImage::ImageLuma32F(float_image);
        }
        DynaImage::ImageLuma32F(img) => {
            return DynaImage::ImageLuma32F(img.clone());
        }
        DynaImage::ImageRgb8(img) => {
            //todo: implement proper srgb to linear conversion
            let rgb_image = img.clone().convert();
            return DynaImage::ImageRgb32F(rgb_image);
        }
        DynaImage::ImageRgb32F(img) => {
            return DynaImage::ImageRgb32F(img.clone());
        }
    }
}

fn resize_image_for_purpose(image: DynaImage, purpose: TexturePurpose) -> DynaImage {
    match purpose {
        TexturePurpose::Render => image,
        TexturePurpose::Display | TexturePurpose::DisplaySrgb => {
            let resized = image.resize_exact(
                DISPLAY_SIZE,
                DISPLAY_SIZE,
                image::imageops::FilterType::CatmullRom,
            );
            return resized;
        }
        TexturePurpose::Icon | TexturePurpose::IconSrgb => {
            let resized = image.resize_exact(
                ICON_SIZE,
                ICON_SIZE,
                image::imageops::FilterType::CatmullRom,
            );
            return resized;
        }
    }
}

fn get_dependent_image(
    textue: &Texture,
    dependencies: &HashMap<String, Arc<RwLock<DynaImage>>>,
    key: &str,
) -> Option<Arc<RwLock<DynaImage>>> {
    if let Some(image) = dependencies.get(key) {
        return Some(image.clone());
    }
    if let Some(image) = get_color_texture_image(textue, key) {
        let image = Arc::new(RwLock::new(image));
        return Some(image);
    }
    let name = textue.get_name();
    println!("{} Dependency image not found for key: {}", name, key);
    return None;
}

fn load_imagemap_texture_image(texture: &Texture, purpose: TexturePurpose) -> Option<DynaImage> {
    if let Some(path) = texture.get_fullpath() {
        if let Ok(image) = image::open(path) {
            todo!("optimize: avoid redundant conversion if the image is already float type");
            let image = resize_image_for_purpose(image, purpose);
            //return Some(image);
        }
    }
    return None;
}

fn render_constant_texture_image(texture: &Texture) -> Option<DynaImage> {
    if let Some(color_image) = get_color_texture_image(texture, "value") {
        return Some(color_image);
    } else {
        // Default to white if color not found
        let color = image::Rgb([1.0, 1.0, 1.0]);
        let image_buffer = image::ImageBuffer::from_pixel(1, 1, color);
        return Some(DynaImage::ImageRgb32F(image_buffer));
    }
}

fn mix_texture(tex1: &DynaImage, tex2: &DynaImage, amount: &DynaImage) -> DynaImage {
    let dim1 = tex1.dimensions();
    let dim2 = tex2.dimensions();
    let dim3 = amount.dimensions();
    let dimf = (
        dim1.0.max(dim2.0).max(dim3.0),
        dim1.1.max(dim2.1).max(dim3.1),
    );

    let tex1 = tex1.resize(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let tex2 = tex2.resize(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let amount = amount.resize(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);

    todo!("optimize: avoid redundant conversion if the image is already float type");

    let tex1 = convert_to_linear_float_image(&tex1);
    let tex2 = convert_to_linear_float_image(&tex2);
    let amount = convert_to_linear_float_image(&amount);
    /* 
    let mut image_buffer = image::ImageBuffer::new(dimf.0, dimf.1);
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let a = amount.get_pixel(x, y)[0]; // Assuming amount is a grayscale image
        let r = p1[0] * (1.0 - a) + p2[0] * a;
        let g = p1[1] * (1.0 - a) + p2[1] * a;
        let b = p1[2] * (1.0 - a) + p2[2] * a;
        *pixel = image::Rgb([r, g, b]);
    }
    return DynaImage::ImageRgb32F(image_buffer);
    */
}

fn render_mix_texture_image(
    texture: &Texture,
    dependencies: &HashMap<String, Arc<RwLock<DynaImage>>>,
) -> Option<DynaImage> {
    let tex1 = get_dependent_image(texture, dependencies, "tex1")?;
    let tex2 = get_dependent_image(texture, dependencies, "tex2")?;
    let amount = get_dependent_image(texture, dependencies, "amount")?;
    let image = mix_texture(
        &tex1.read().unwrap(),
        &tex2.read().unwrap(),
        &amount.read().unwrap(),
    );
    return Some(image);
}

fn scale_texture(tex1: &DynaImage, tex2: &DynaImage) -> DynaImage {
    let dim1 = tex1.dimensions();
    let dim2 = tex2.dimensions();
    let dimf = (dim1.0.max(dim2.0), dim1.1.max(dim2.1));

    let tex1 = tex1.resize(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let tex2 = tex2.resize(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);

    todo!("optimize: avoid redundant conversion if the image is already float type");

    let tex1 = convert_to_linear_float_image(&tex1);
    let tex2 = convert_to_linear_float_image(&tex2);
    /* 
    let mut image_buffer = image::ImageBuffer::new(dimf.0, dimf.1);
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let r = p1[0] * p2[0];
        let g = p1[1] * p2[1];
        let b = p1[2] * p2[2];
        *pixel = image::Rgb([r, g, b]);
    }
    return DynaImage::ImageRgb32F(image_buffer);
    */
}

fn render_scale_texture_image(
    texture: &Texture,
    dependencies: &HashMap<String, Arc<RwLock<DynaImage>>>,
) -> Option<DynaImage> {
    let tex1 = get_dependent_image(texture, dependencies, "tex1")?;
    let tex2 = get_dependent_image(texture, dependencies, "tex2")?;
    let image = scale_texture(&tex1.read().unwrap(), &tex2.read().unwrap());
    return Some(image);
}

pub fn render_texture_image(
    texture: &Texture,
    dependencies: &HashMap<String, Arc<RwLock<DynaImage>>>,
    purpose: TexturePurpose,
) -> Option<DynaImage> {
    let texture_type = texture.get_type();
    match texture_type.as_str() {
        "imagemap" => {
            return load_imagemap_texture_image(texture, purpose);
        }
        "constant" => {
            return render_constant_texture_image(texture);
        }
        "mix" => {
            return render_mix_texture_image(texture, dependencies);
        }
        "scale" => {
            return render_scale_texture_image(texture, dependencies);
        }
        _ => {
            return None; // Placeholder return
        }
    }
}
