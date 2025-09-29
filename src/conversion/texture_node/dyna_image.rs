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
    ImageLuma32F(image::ImageBuffer<image::Luma<f32>, Vec<f32>>),
    ImageRgb32F(image::ImageBuffer<image::Rgb<f32>, Vec<f32>>),
}
