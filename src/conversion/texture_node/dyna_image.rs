use image::{buffer::ConvertBuffer, imageops};
//pub type Rgb32FImage = ImageBuffer<Rgb<f32>, Vec<f32>>;

fn linear_to_srgb(value: f32) -> u8 {
    if value <= 0.0031308 {
        (value * 12.92 * 255.0).round() as u8
    } else {
        ((1.055 * value.powf(1.0 / 2.4) - 0.055) * 255.0).round() as u8
    }
}

fn srgb_to_linear(value: u8) -> f32 {
    let v = value as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

// Inner representation of images used in texture cache
// Only f32 images are supported for now
// Luma32F and Rgb32F are supported
// Other image types are converted to these types when loaded
// We wanted to use DynamicImage here, but it does not support ImageLuma32F - single channel f32 image
// so we define our own enum here
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum DynaImage {
    ImageLuma8(image::ImageBuffer<image::Luma<u8>, Vec<u8>>),
    ImageRgb8(image::ImageBuffer<image::Rgb<u8>, Vec<u8>>),
    ImageLuma32F(image::ImageBuffer<image::Luma<f32>, Vec<f32>>),
    ImageRgb32F(image::ImageBuffer<image::Rgb<f32>, Vec<f32>>),
}

impl DynaImage {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            DynaImage::ImageLuma8(img) => img.dimensions(),
            DynaImage::ImageRgb8(img) => img.dimensions(),
            DynaImage::ImageLuma32F(img) => img.dimensions(),
            DynaImage::ImageRgb32F(img) => img.dimensions(),
        }
    }

    #[must_use]
    pub fn resize(&self, nwidth: u32, nheight: u32, filter: imageops::FilterType) -> DynaImage {
        match self {
            DynaImage::ImageLuma8(p) => {
                DynaImage::ImageLuma8(imageops::resize(p, nwidth, nheight, filter))
            }
            DynaImage::ImageRgb8(p) => {
                DynaImage::ImageRgb8(imageops::resize(p, nwidth, nheight, filter))
            }
            DynaImage::ImageLuma32F(p) => {
                DynaImage::ImageLuma32F(imageops::resize(p, nwidth, nheight, filter))
            }
            DynaImage::ImageRgb32F(p) => {
                DynaImage::ImageRgb32F(imageops::resize(p, nwidth, nheight, filter))
            }
        }
    }

    pub fn resize_exact(
        &self,
        nwidth: u32,
        nheight: u32,
        filter: imageops::FilterType,
    ) -> DynaImage {
        return self.resize(nwidth, nheight, filter);
    }

    pub fn to_rgb8(&self) -> image::RgbImage {
        match self {
            DynaImage::ImageLuma8(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb8(img) => img.clone(),
            DynaImage::ImageLuma32F(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb32F(img) => {
                let width = img.width();
                let height = img.height();
                let mut result_image = image::ImageBuffer::new(width, height);
                for (x, y, pixel) in img.enumerate_pixels() {
                    result_image.put_pixel(
                        x,
                        y,
                        image::Rgb([
                            linear_to_srgb(pixel[0]),
                            linear_to_srgb(pixel[1]),
                            linear_to_srgb(pixel[2]),
                        ]),
                    );
                }
                return result_image;
            }
        }
    }

    pub fn to_rgb32f(&self) -> image::Rgb32FImage {
        match self {
            DynaImage::ImageLuma8(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb8(img) => {
                let mut result_image = image::ImageBuffer::new(img.width(), img.height());
                for (x, y, pixel) in img.enumerate_pixels() {
                    result_image.put_pixel(
                        x,
                        y,
                        image::Rgb([
                            srgb_to_linear(pixel[0]),
                            srgb_to_linear(pixel[1]),
                            srgb_to_linear(pixel[2]),
                        ]),
                    );
                }
                return result_image;
            }
            DynaImage::ImageLuma32F(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb32F(img) => img.clone(),
        }
    }

    pub fn to_rgba32f(&self) -> image::Rgba32FImage {
        match self {
            DynaImage::ImageLuma8(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb8(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageLuma32F(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb32F(img) => {
                return img.clone().convert();
            }
        }
    }
}
