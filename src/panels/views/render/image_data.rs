

#[derive(Debug, Clone)]
pub struct ImageData {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub channel_names: Vec<String>,
    pub data: Vec<f32>,
    pub tiles: Vec<(usize, usize, usize, usize)>, // (x0, y0, x1, y1)
}

impl ImageData {
    pub fn new(name: String, width: usize, height: usize, channel_names: &Vec<String>) -> Self {
        let channel_count = channel_names.len();
        let data_length = width * height * channel_count;
        let data = vec![0.0; data_length]; // Initialize with zeros
        Self {
            name,
            width,
            height,
            channel_names: channel_names.clone(),
            data: data,        // Initialize with empty data
            tiles: Vec::new(), // Initialize with empty tiles
        }
    }
}
