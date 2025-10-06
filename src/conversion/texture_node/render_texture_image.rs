use super::texture_node::TexturePurpose;
use crate::conversion::spectrum::Spectrum;
use crate::model::base::Property;
use crate::model::scene::Texture;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use crate::conversion::texture_node::DynaImage;
use image::ImageBuffer;
use image::buffer::ConvertBuffer as _;

const ICON_SIZE: u32 = 64;
const DISPLAY_SIZE: u32 = 256;
const RENDER_SIZE: u32 = 1024;

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

fn convert_to_linear_float_image(image: &DynaImage) -> DynaImage {
    match image {
        DynaImage::ImageLuma8(img) => {
            let float_image: ImageBuffer<image::Luma<f32>, Vec<f32>> = img.clone().convert();
            return DynaImage::ImageLuma32F(float_image);
        }
        DynaImage::ImageLuma32F(img) => {
            return DynaImage::ImageLuma32F(img.clone());
        }
        DynaImage::ImageRgb8(_) => {
            return DynaImage::ImageRgb32F(image.to_rgb32f()); // convert_to_float_image_buffer(image);
        }
        DynaImage::ImageRgb32F(img) => {
            return DynaImage::ImageRgb32F(img.clone());
        }
    }
}

fn resize_image_for_purpose(
    image: image::DynamicImage,
    purpose: TexturePurpose,
) -> image::DynamicImage {
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

fn unify_channels(images: &HashMap<String, Arc<DynaImage>>) -> HashMap<String, Arc<DynaImage>> {
    let mut should_be_color = false;
    for (_key, image) in images.iter() {
        match image.as_ref() {
            DynaImage::ImageRgb8(_) => {
                should_be_color = true;
                break;
            }
            DynaImage::ImageRgb32F(_) => {
                should_be_color = true;
                break;
            }
            _ => {}
        }
    }
    if should_be_color {
        let mut new_images = HashMap::new();
        for (key, image) in images.iter() {
            match image.as_ref() {
                DynaImage::ImageLuma8(img) => {
                    let color_image: ImageBuffer<image::Rgb<u8>, Vec<u8>> = img.clone().convert();
                    let color_image = DynaImage::ImageRgb8(color_image);
                    new_images.insert(key.clone(), Arc::new(color_image));
                }
                DynaImage::ImageLuma32F(img) => {
                    let color_image: ImageBuffer<image::Rgb<f32>, Vec<f32>> = img.clone().convert();
                    let color_image = DynaImage::ImageRgb32F(color_image);
                    new_images.insert(key.clone(), Arc::new(color_image));
                }
                _ => {
                    new_images.insert(key.clone(), image.clone());
                }
            }
        }
        return new_images;
    } else {
        return images.clone();
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
    return None;
}

fn load_imagemap_texture_image(texture: &Texture, purpose: TexturePurpose) -> Option<DynaImage> {
    if let Some(path) = texture.get_fullpath() {
        if let Ok(image) = image::open(path) {
            let image = resize_image_for_purpose(image, purpose);
            match image {
                image::DynamicImage::ImageLuma8(img) => {
                    return Some(DynaImage::ImageLuma8(img));
                }
                image::DynamicImage::ImageRgb8(img) => {
                    return Some(DynaImage::ImageRgb8(img));
                }
                image::DynamicImage::ImageRgb32F(img) => {
                    return Some(DynaImage::ImageRgb32F(img.clone()));
                }
                image::DynamicImage::ImageRgba32F(img) => {
                    let img = img.convert();
                    return Some(DynaImage::ImageLuma32F(img));
                }
                _ => {
                    let img = image.to_rgb8();
                    return Some(DynaImage::ImageRgb8(img));
                }
            }
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

fn mix_pixel_rgb(
    p1: &image::Rgb<f32>,
    p2: &image::Rgb<f32>,
    a: &image::Rgb<f32>,
) -> image::Rgb<f32> {
    let mut result = p1.clone();
    for i in 0..3 {
        let c1: f32 = p1[i as usize];
        let c2: f32 = p2[i as usize];
        let a: f32 = a[i as usize]; // Assuming amount is a grayscale image
        let c = c1 * (1.0 - a) + c2 * a;
        result[i as usize] = c;
    }
    return result;
}

fn mix_texture_rgb(
    tex1: &image::ImageBuffer<image::Rgb<f32>, Vec<f32>>,
    tex2: &image::ImageBuffer<image::Rgb<f32>, Vec<f32>>,
    amount: &image::ImageBuffer<image::Rgb<f32>, Vec<f32>>,
) -> image::ImageBuffer<image::Rgb<f32>, Vec<f32>> {
    let mut image_buffer = image::ImageBuffer::new(tex1.width(), tex1.height());
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let a = amount.get_pixel(x, y); // Assuming amount is a grayscale image
        let c = mix_pixel_rgb(p1, p2, a);
        *pixel = c.into();
    }
    return image_buffer;
}

fn mix_texture_float(
    tex1: &image::ImageBuffer<image::Luma<f32>, Vec<f32>>,
    tex2: &image::ImageBuffer<image::Luma<f32>, Vec<f32>>,
    amount: &image::ImageBuffer<image::Luma<f32>, Vec<f32>>,
) -> image::ImageBuffer<image::Luma<f32>, Vec<f32>> {
    let mut image_buffer = image::ImageBuffer::new(tex1.width(), tex1.height());
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let a = amount.get_pixel(x, y); // Assuming amount is a grayscale image
        let c = p1[0] * (1.0 - a[0]) + p2[0] * a[0];
        *pixel = image::Luma([c]);
    }
    return image_buffer;
}

fn mix_texture(tex1: &DynaImage, tex2: &DynaImage, amount: &DynaImage) -> Option<DynaImage> {
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

    let tex1 = convert_to_linear_float_image(&tex1);
    let tex2 = convert_to_linear_float_image(&tex2);
    let amount = convert_to_linear_float_image(&amount);
    let mut image_map = HashMap::new();
    image_map.insert("tex1".to_string(), Arc::new(tex1));
    image_map.insert("tex2".to_string(), Arc::new(tex2));
    image_map.insert("amount".to_string(), Arc::new(amount));
    let image_map = unify_channels(&image_map);
    let tex1 = image_map.get("tex1").unwrap().clone();
    let tex2 = image_map.get("tex2").unwrap().clone();
    let amount = image_map.get("amount").unwrap().clone();

    if let (
        DynaImage::ImageRgb32F(tex1),
        DynaImage::ImageRgb32F(tex2),
        DynaImage::ImageRgb32F(amount),
    ) = (tex1.as_ref(), tex2.as_ref(), amount.as_ref())
    {
        let image_buffer = mix_texture_rgb(&tex1, &tex2, &amount);
        return Some(DynaImage::ImageRgb32F(image_buffer));
    } else if let (
        DynaImage::ImageLuma32F(tex1),
        DynaImage::ImageLuma32F(tex2),
        DynaImage::ImageLuma32F(amount),
    ) = (tex1.as_ref(), tex2.as_ref(), amount.as_ref())
    {
        let image_buffer = mix_texture_float(&tex1, &tex2, &amount);
        return Some(DynaImage::ImageLuma32F(image_buffer));
    }
    return None;
}

fn render_mix_texture_image(
    texture: &Texture,
    dependencies: &HashMap<String, Arc<RwLock<DynaImage>>>,
) -> Option<DynaImage> {
    let tex1 = get_dependent_image(texture, dependencies, "tex1")?;
    let tex2 = get_dependent_image(texture, dependencies, "tex2")?;
    let amount = get_dependent_image(texture, dependencies, "amount")?;
    return mix_texture(
        &tex1.read().unwrap(),
        &tex2.read().unwrap(),
        &amount.read().unwrap(),
    );
}

fn scale_pixel_rgb(p1: &image::Rgb<f32>, p2: &image::Rgb<f32>) -> image::Rgb<f32> {
    let mut result = p1.clone();
    for i in 0..3 {
        let c = p1[i as usize] * p2[i as usize];
        result[i as usize] = c;
    }
    return result;
}

fn scale_pixel_float(p1: &image::Luma<f32>, p2: &image::Luma<f32>) -> image::Luma<f32> {
    let c = p1[0] * p2[0];
    return image::Luma([c]);
}

fn scale_texture_float(
    tex1: &image::ImageBuffer<image::Luma<f32>, Vec<f32>>,
    tex2: &image::ImageBuffer<image::Luma<f32>, Vec<f32>>,
) -> image::ImageBuffer<image::Luma<f32>, Vec<f32>> {
    let mut image_buffer = image::ImageBuffer::new(tex1.width(), tex1.height());
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let c = scale_pixel_float(p1, p2);
        *pixel = c.into();
    }
    return image_buffer;
}

fn scale_texture_rgb(
    tex1: &image::ImageBuffer<image::Rgb<f32>, Vec<f32>>,
    tex2: &image::ImageBuffer<image::Rgb<f32>, Vec<f32>>,
) -> image::ImageBuffer<image::Rgb<f32>, Vec<f32>> {
    let mut image_buffer = image::ImageBuffer::new(tex1.width(), tex1.height());
    for (x, y, pixel) in image_buffer.enumerate_pixels_mut() {
        let p1 = tex1.get_pixel(x, y);
        let p2 = tex2.get_pixel(x, y);
        let c = scale_pixel_rgb(p1, p2);
        *pixel = c.into();
    }
    return image_buffer;
}

fn scale_texture(tex1: &DynaImage, tex2: &DynaImage) -> Option<DynaImage> {
    let dim1 = tex1.dimensions();
    let dim2 = tex2.dimensions();
    let dimf = (dim1.0.max(dim2.0), dim1.1.max(dim2.1));

    let tex1 = tex1.resize(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);
    let tex2 = tex2.resize(dimf.0, dimf.1, image::imageops::FilterType::Lanczos3);

    let tex1 = convert_to_linear_float_image(&tex1);
    let tex2 = convert_to_linear_float_image(&tex2);
    let mut image_map = HashMap::new();
    image_map.insert("tex1".to_string(), Arc::new(tex1));
    image_map.insert("tex2".to_string(), Arc::new(tex2));
    let image_map = unify_channels(&image_map);
    let tex1 = image_map.get("tex1").unwrap().clone();
    let tex2 = image_map.get("tex2").unwrap().clone();

    if let (DynaImage::ImageRgb32F(tex1), DynaImage::ImageRgb32F(tex2)) =
        (tex1.as_ref(), tex2.as_ref())
    {
        let image_buffer = scale_texture_rgb(&tex1, &tex2);
        return Some(DynaImage::ImageRgb32F(image_buffer));
    } else if let (DynaImage::ImageLuma32F(tex1), DynaImage::ImageLuma32F(tex2)) =
        (tex1.as_ref(), tex2.as_ref())
    {
        let image_buffer = scale_texture_float(&tex1, &tex2);
        return Some(DynaImage::ImageLuma32F(image_buffer));
    }
    return None;
}

fn render_scale_texture_image(
    texture: &Texture,
    dependencies: &HashMap<String, Arc<RwLock<DynaImage>>>,
) -> Option<DynaImage> {
    let tex1 = get_dependent_image(texture, dependencies, "tex1")?;
    let tex2 = get_dependent_image(texture, dependencies, "tex2")?;
    return scale_texture(&tex1.read().unwrap(), &tex2.read().unwrap());
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
