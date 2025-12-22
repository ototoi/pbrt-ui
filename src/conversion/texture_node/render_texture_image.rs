use super::texture_node::TextureSizeType;
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

// Perlin Noise Implementation for FBM texture
// Based on pbrt-r3 noise implementation
const NOISEPERMSIZE: u32 = 256;
// FBM frequency scaling (lacunarity) - determines how much the frequency increases per octave
const FBM_LACUNARITY: f32 = 1.99;
// Default UV scaling for FBM texture generation
const FBM_DEFAULT_SCALE: f32 = 4.0;

const NOISEPERM: [u32; 512] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
    // Second half (duplicate)
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

#[inline]
fn noise_weight(t: f32) -> f32 {
    let t3 = t * t * t;
    let t4 = t3 * t;
    6.0 * t4 * t - 15.0 * t4 + 10.0 * t3
}

#[inline]
fn grad(x: u32, y: u32, z: u32, dx: f32, dy: f32, dz: f32) -> f32 {
    // SAFETY: x, y, z must be masked with (NOISEPERMSIZE - 1) by caller to ensure valid array access
    let h = NOISEPERM[NOISEPERM[NOISEPERM[x as usize] as usize + y as usize] as usize + z as usize];
    let h = h & 15;
    let u = if h < 8 || h == 12 || h == 13 { dx } else { dy };
    let v = if h < 4 || h == 12 || h == 13 { dy } else { dz };
    (if (h & 1) != 0 { -u } else { u }) + (if (h & 2) != 0 { -v } else { v })
}

#[inline]
fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

fn noise(x: f32, y: f32, z: f32) -> f32 {
    // Compute noise cell coordinates and offsets
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let iz = z.floor() as i32;
    let dx = x - ix as f32;
    let dy = y - iy as f32;
    let dz = z - iz as f32;

    // Compute gradient weights
    let ix = (ix as u32) & (NOISEPERMSIZE - 1);
    let iy = (iy as u32) & (NOISEPERMSIZE - 1);
    let iz = (iz as u32) & (NOISEPERMSIZE - 1);

    let w000 = grad(ix, iy, iz, dx, dy, dz);
    let w100 = grad(ix + 1, iy, iz, dx - 1.0, dy, dz);
    let w010 = grad(ix, iy + 1, iz, dx, dy - 1.0, dz);
    let w110 = grad(ix + 1, iy + 1, iz, dx - 1.0, dy - 1.0, dz);
    let w001 = grad(ix, iy, iz + 1, dx, dy, dz - 1.0);
    let w101 = grad(ix + 1, iy, iz + 1, dx - 1.0, dy, dz - 1.0);
    let w011 = grad(ix, iy + 1, iz + 1, dx, dy - 1.0, dz - 1.0);
    let w111 = grad(ix + 1, iy + 1, iz + 1, dx - 1.0, dy - 1.0, dz - 1.0);

    // Compute trilinear interpolation of weights
    let wx = noise_weight(dx);
    let wy = noise_weight(dy);
    let wz = noise_weight(dz);
    let x00 = lerp(wx, w000, w100);
    let x10 = lerp(wx, w010, w110);
    let x01 = lerp(wx, w001, w101);
    let x11 = lerp(wx, w011, w111);
    let y0 = lerp(wy, x00, x10);
    let y1 = lerp(wy, x01, x11);
    lerp(wz, y0, y1)
}

fn fbm(px: f32, py: f32, pz: f32, omega: f32, octaves: u32) -> f32 {
    // Compute sum of octaves of noise for FBm
    let mut sum = 0.0;
    let mut lambda = 1.0;
    let mut o = 1.0;
    for _ in 0..octaves {
        sum += o * noise(lambda * px, lambda * py, lambda * pz);
        lambda *= FBM_LACUNARITY;
        o *= omega;
    }
    sum
}

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

fn resize_image(image: &DynaImage, width: u32, height: u32) -> DynaImage {
    if (1, 1) == image.dimensions() {
        let mut resized = image.resize(width, height, image::imageops::FilterType::Nearest);
        if let DynaImage::ImageLuma32F(dst) = &mut resized {
            if let DynaImage::ImageLuma32F(src) = image {
                let vv = src.get_pixel(0, 0)[0];
                for y in 0..height {
                    for x in 0..width {
                        dst.put_pixel(x, y, image::Luma([vv]));
                    }
                }
            }
        }
        return resized;
    } else {
        let resized = image.resize(width, height, image::imageops::FilterType::CatmullRom);
        return resized;
    }
}

fn resize_image_for_size_type(
    image: image::DynamicImage,
    size_type: TextureSizeType,
) -> image::DynamicImage {
    match size_type {
        TextureSizeType::Render => image,
        TextureSizeType::Display => {
            let resized = image.resize_exact(
                DISPLAY_SIZE,
                DISPLAY_SIZE,
                image::imageops::FilterType::CatmullRom,
            );
            return resized;
        }
        TextureSizeType::Icon => {
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

fn load_imagemap_texture_image(texture: &Texture, size_type: TextureSizeType) -> Option<DynaImage> {
    // let scale = get_float(texture.as_property_map(), "scale").unwrap_or(1.0); --- IGNORE ---
    // if scale != 1.0 { --- IGNORE ---
    //     println!("Imagemap scale other than 1.0 is not supported yet."); --- IGNORE ---
    // } --- IGNORE ---
    if let Some(path) = texture.get_fullpath() {
        if let Ok(image) = image::open(path) {
            let image = resize_image_for_size_type(image, size_type);
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

    let tex1 = resize_image(&tex1, dimf.0, dimf.1);
    let tex2 = resize_image(&tex2, dimf.0, dimf.1);
    let amount = resize_image(&amount, dimf.0, dimf.1);

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

fn scale_pixel_helper(p1: f32, p2: f32) -> f32 {
    if p2 >= 0.0 {
        let c = p1 * p2;
        return c;
    } else {
        let c = ((1.0 - p1) * -p2).max(0.0);
        return c;
    }
}

fn scale_pixel_rgb(p1: &image::Rgb<f32>, p2: &image::Rgb<f32>) -> image::Rgb<f32> {
    let mut result = p1.clone();
    for i in 0..3 {
        let c = scale_pixel_helper(p1[i as usize], p2[i as usize]);
        result[i as usize] = c;
    }
    return result;
}

fn scale_pixel_float(p1: &image::Luma<f32>, p2: &image::Luma<f32>) -> image::Luma<f32> {
    let c = scale_pixel_helper(p1[0], p2[0]);
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

    let tex1 = resize_image(&tex1, dimf.0, dimf.1);
    let tex2 = resize_image(&tex2, dimf.0, dimf.1);

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

fn render_fbm_texture_image(texture: &Texture, size_type: TextureSizeType) -> Option<DynaImage> {
    // Get parameters from property map
    let props = texture.as_property_map();
    
    // Read octaves parameter (default: 8, as per pbrt-r3 reference)
    let octaves = if let Some(Property::Ints(v)) = props.get("integer octaves") {
        if !v.is_empty() {
            v[0] as u32
        } else {
            8
        }
    } else {
        8
    };
    
    // Read roughness/omega parameter (default: 0.5, as per pbrt-r3 reference)
    let roughness = if let Some(Property::Floats(v)) = props.get("float roughness") {
        if !v.is_empty() {
            v[0]
        } else {
            0.5
        }
    } else {
        0.5
    };
    
    // Determine output size based on size_type
    let size = match size_type {
        TextureSizeType::Icon => ICON_SIZE,
        TextureSizeType::Display => DISPLAY_SIZE,
        TextureSizeType::Render => RENDER_SIZE,
    };
    
    // Generate FBM texture
    let mut image_buffer = image::ImageBuffer::new(size, size);
    
    // Scale factor for UV coordinates
    let scale = FBM_DEFAULT_SCALE;
    
    for y in 0..size {
        for x in 0..size {
            // Normalize coordinates to [0, 1] and scale
            let u = (x as f32 / size as f32) * scale;
            let v = (y as f32 / size as f32) * scale;
            let w = 0.0; // Z coordinate
            
            // Compute FBM noise value
            let noise_value = fbm(u, v, w, roughness, octaves);
            
            // Normalize to [0, 1] range
            // FBM typically produces values in roughly [-1, 1] range
            let normalized = (noise_value + 1.0) * 0.5;
            let clamped = normalized.clamp(0.0, 1.0);
            
            image_buffer.put_pixel(x, y, image::Luma([clamped]));
        }
    }
    
    Some(DynaImage::ImageLuma32F(image_buffer))
}

pub fn render_texture_image(
    texture: &Texture,
    dependencies: &HashMap<String, Arc<RwLock<DynaImage>>>,
    size_type: TextureSizeType,
) -> Option<DynaImage> {
    let texture_type = texture.get_type();
    match texture_type.as_str() {
        "imagemap" => {
            return load_imagemap_texture_image(texture, size_type);
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
        "fbm" => {
            return render_fbm_texture_image(texture, size_type);
        }
        _ => {
            return None; // Placeholder return
        }
    }
}
