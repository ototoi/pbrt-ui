use std::convert;

use image::{buffer::ConvertBuffer, imageops};
//pub type Rgb32FImage = ImageBuffer<Rgb<f32>, Vec<f32>>;

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

    pub fn resize_exact(&self, nwidth: u32, nheight: u32, filter: imageops::FilterType) -> DynaImage {
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
                //todo: implement proper linear to srgb conversion
                return img.clone().convert();
            }
        }
    }

    pub fn to_rgb32f(&self) -> image::Rgb32FImage {
        match self {
            DynaImage::ImageLuma8(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb8(img) => {
                //todo: implement proper srgb to linear conversion
                return img.clone().convert();
            }
            DynaImage::ImageLuma32F(img) => {
                return img.clone().convert();
            }
            DynaImage::ImageRgb32F(img) => img.clone(),
        }
    }
}
