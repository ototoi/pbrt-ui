use super::image_data::ImageData;
use crate::error::PbrtError;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
enum DisplayDirective {
    _ReloadImage = 1,
    _CloseImage = 2,
    _UpdateImage = 3,
    CreateImage = 4,
    _UpdateImageV2 = 5,
    UpdateImageV3 = 6,
    _OpenImage = 7,
    _VectorGraphics = 8,
    //
    CloseServer = 129,
}

#[derive(Debug, Clone)]
struct CreateImageData {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub channel_names: Vec<String>,
}

struct UpdateImageData {
    pub name: String,
    pub channel_names: Vec<String>,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>, // Placeholder for image data
}

struct ImageReceiverCore {
    images: HashMap<String, Arc<Mutex<ImageData>>>,
}

impl ImageReceiverCore {
    fn new() -> Self {
        Self {
            images: HashMap::new(),
        }
    }

    fn create_image(&mut self, create_image: CreateImageData) {
        let image_data = ImageData::new(
            create_image.name.clone(),
            create_image.width as usize,
            create_image.height as usize,
            &create_image.channel_names,
        );
        let image_data = Arc::new(Mutex::new(image_data));
        self.images.insert(create_image.name, image_data);
    }

    fn update_image(&mut self, update_image: UpdateImageData) {
        // Handle the UpdateImage directive
        //println!("UpdateImage: name={}", update_image.name);
        if let Some(image) = self.images.get(&update_image.name) {
            let x0 = update_image.x as usize;
            let y0 = update_image.y as usize;
            let x1 = x0 + update_image.width as usize;
            let y1 = y0 + update_image.height as usize;
            let n_channels = update_image.channel_names.len();

            let mut image_data = image.lock().unwrap();
            for y in y0..y1 {
                for x in x0..x1 {
                    let index = (y * image_data.width + x) * n_channels;
                    for c in 0..n_channels {
                        let dx = x - x0;
                        let dy = y - y0;
                        let dw = update_image.width as usize;
                        let data_index = (dy * dw + dx) * n_channels + c;
                        if data_index < update_image.data.len() {
                            image_data.data[index + c] = update_image.data[data_index];
                        } else {
                            //println!("Data index out of bounds: {}", data_index);
                        }
                    }
                }
            }
            image_data.tiles.push((x0, y0, x1, y1)); // Store the updated tile
        }
    }
}

pub struct ImageReceiver {
    core: Arc<Mutex<ImageReceiverCore>>,
    handle: Option<(String, Sender<i32>, thread::JoinHandle<()>)>,
}

fn read_bytes(stream: &mut TcpStream) -> Result<Vec<u8>, PbrtError> {
    let mut length_buffer = [0; 4];
    let l = stream
        .peek(&mut length_buffer)
        .map_err(|e| PbrtError::error(&format!("Failed to read length: {}", e)))?;
    if l != 4 {
        return Ok(vec![]); // No data to read
    }
    let length = u32::from_le_bytes(length_buffer) as usize;
    if length == 0 {
        //println!("Received length is 0, no data to read.");
        return Ok(vec![]); // No data to read
    }
    //println!("Reading {} bytes of data", length);
    let mut buffer = vec![0; length];
    stream
        .read_exact(&mut buffer)
        .map_err(|e| PbrtError::error(&format!("Failed to read data: {}", e)))?;
    //assert!(l == length, "Expected to read {} bytes, but read {}", length, l);
    Ok(buffer)
}

fn decode_create_image(data: &[u8]) -> Result<CreateImageData, PbrtError> {
    if data.len() < 4 {
        return Err(PbrtError::error("Data too short for CreateImage directive"));
    }
    let mut length_buffer = [0; 4];
    length_buffer.copy_from_slice(&data[0..4]);
    let _length = u32::from_le_bytes(length_buffer);
    if data.len() < 5 {
        return Err(PbrtError::error(
            "Data too short for CreateImage directive with name",
        ));
    }
    let directive: u8 = data[4];
    assert!(
        directive == DisplayDirective::CreateImage as u8,
        "Expected directive 4 for CreateImage, got {}",
        directive
    );
    if data.len() < 9 {
        return Err(PbrtError::error(
            "Data too short for CreateImage directive with width and height",
        ));
    }
    let _grab_focus = data[5];
    let offset = 6;
    let mut name_buffer = Vec::new();
    for b in data[offset..].iter() {
        if *b == 0 {
            break; // Null terminator found
        }
        name_buffer.push(*b);
    }
    let name = String::from_utf8(name_buffer.clone())
        .map_err(|e| PbrtError::error(&format!("Failed to decode image name: {}", e)))?;
    let offset = offset + name_buffer.len() + 1; // +1 for null terminator
    if data.len() < offset + 8 {
        return Err(PbrtError::error(
            "Data too short for CreateImage directive with width and height",
        ));
    }
    let width = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    let height = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
    let offset = offset + 8; // Move past width and height
    if data.len() < offset + 4 {
        return Err(PbrtError::error(
            "Data too short for CreateImage directive with channel names",
        ));
    }
    let n_channels = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    let mut offset = offset + 4; // Move past number of channels
    let mut channel_names = Vec::new();
    for _ in 0..n_channels {
        let mut channel_name_buffer = Vec::new();
        if offset >= data.len() {
            return Err(PbrtError::error(
                "Data too short for CreateImage directive with channel names",
            ));
        }
        for b in data[offset..].iter() {
            if *b == 0 {
                let channel_name = String::from_utf8(channel_name_buffer.clone()).map_err(|e| {
                    PbrtError::error(&format!("Failed to decode channel name: {}", e))
                })?;
                channel_names.push(channel_name);
                break; // Null terminator found
            }
            channel_name_buffer.push(*b);
        }
        offset += channel_name_buffer.len() + 1; // Move past channel name and null terminator
    }
    Ok(CreateImageData {
        name,
        width,
        height,
        channel_names,
    })
}

fn decode_update_image(data: &[u8]) -> Result<UpdateImageData, PbrtError> {
    // Implement the decoding logic for UpdateImage directive
    // This is a placeholder implementation
    if data.len() < 5 {
        return Err(PbrtError::error("Data too short for UpdateImage directive"));
    }
    let directive: u8 = data[4];
    assert!(
        directive == DisplayDirective::UpdateImageV3 as u8,
        "Expected directive 6 for UpdateImage, got {}",
        directive
    );
    let _grab_focus = data[5];
    let offset = 6;
    let mut name_buffer = Vec::new();
    for b in data[offset..].iter() {
        if *b == 0 {
            break; // Null terminator found
        }
        name_buffer.push(*b);
    }
    let name = String::from_utf8(name_buffer.clone())
        .map_err(|e| PbrtError::error(&format!("Failed to decode image name: {}", e)))?;
    let offset = offset + name_buffer.len() + 1; // +1 for null terminator
    if data.len() < offset + 4 {
        return Err(PbrtError::error(
            "Data too short for CreateImage directive with channel names",
        ));
    }
    let n_channels = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    let mut offset = offset + 4; // Move past number of channels
    let mut channel_names = Vec::new();
    for _ in 0..n_channels {
        let mut channel_name_buffer = Vec::new();
        if offset >= data.len() {
            return Err(PbrtError::error(
                "Data too short for CreateImage directive with channel names",
            ));
        }
        for b in data[offset..].iter() {
            if *b == 0 {
                let channel_name = String::from_utf8(channel_name_buffer.clone()).map_err(|e| {
                    PbrtError::error(&format!("Failed to decode channel name: {}", e))
                })?;
                channel_names.push(channel_name);
                break; // Null terminator found
            }
            channel_name_buffer.push(*b);
        }
        offset += channel_name_buffer.len() + 1; // Move past channel name and null terminator
    }
    if data.len() < offset + 16 {
        return Err(PbrtError::error(
            "Data too short for UpdateImage directive with x, y, width, height",
        ));
    }
    let x = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    let y = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
    let width = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());
    let height = u32::from_le_bytes(data[offset + 12..offset + 16].try_into().unwrap());
    let offset = offset + 16; // Move past x, y, width, height
    let data_byte_length = (width * height * n_channels) as usize * size_of::<f32>(); // Each channel is 4 bytes (f32)
    if data.len() < offset + data_byte_length {
        return Err(PbrtError::error(
            "Data too short for UpdateImage directive with image data",
        ));
    }
    let mut channel_indices: [i64; 4] = [0; 4];
    for i in 0..n_channels as usize {
        let start = offset + i * size_of::<i64>();
        let end = start + size_of::<i64>();
        if i < 4 {
            channel_indices[i] = i64::from_le_bytes(data[start..end].try_into().unwrap());
        } else {
            channel_indices[i] = 0; // Default to first channel if more than 4 channels
        }
    }
    let offset = offset + n_channels as usize * size_of::<i64>(); // Move past channel indices
    let mut channel_counts: [i64; 4] = [0; 4];
    for i in 0..n_channels as usize {
        let start = offset + i * size_of::<i64>();
        let end = start + size_of::<i64>();
        if i < 4 {
            channel_counts[i] = i64::from_le_bytes(data[start..end].try_into().unwrap());
        } else {
            channel_counts[i] = 0; // Default to first channel if more than 4 channels
        }
    }
    let offset = offset + n_channels as usize * size_of::<i64>(); // Move past channel indices
    let mut image_data = Vec::with_capacity(data_byte_length);
    for i in 0..(width * height * n_channels) as usize {
        let start = offset + i * size_of::<f32>();
        let end = start + size_of::<f32>();
        if end > data.len() {
            return Err(PbrtError::error(
                "Data too short for UpdateImage directive with image data",
            ));
        }
        let value = f32::from_le_bytes(data[start..end].try_into().unwrap());
        image_data.push(value);
    }

    Ok(UpdateImageData {
        name,
        channel_names,
        x,
        y,
        width,
        height,
        data: image_data,
    })
}

fn evaluate_bytes(
    core: &Arc<Mutex<ImageReceiverCore>>,
    data: &[u8],
) -> Result<DisplayDirective, PbrtError> {
    let directive: u8 = data[4];
    //println!("Received directive: {:?}", directive);
    let directive = match directive {
        4 => DisplayDirective::CreateImage,
        6 => DisplayDirective::UpdateImageV3,
        129 => DisplayDirective::CloseServer,
        _ => {
            println!("Unknown directive: {}", directive);
            return Err(PbrtError::error(&format!(
                "Unknown directive: {}",
                directive
            )));
        }
    };

    match directive {
        DisplayDirective::CreateImage => {
            // Handle CreateImage directive
            let create_image = decode_create_image(data)?;
            let mut core = core.lock().unwrap();
            core.create_image(create_image);
        }
        DisplayDirective::UpdateImageV3 => {
            // Handle UpdateImage directive
            let update_image = decode_update_image(data)?;
            let mut core = core.lock().unwrap();
            core.update_image(update_image);
        }
        _ => {
            // Handle other directives
            println!("Handling other directive: {:?}", directive);
            // You can add more handling logic for other directives as needed
        }
    }

    return Ok(directive);
}

fn send_close_server(hostname: &str) -> Result<(), PbrtError> {
    let mut stream = TcpStream::connect(hostname)
        .map_err(|e| PbrtError::error(&format!("Failed to connect to {}: {}", hostname, e)))?;
    let directive = DisplayDirective::CloseServer as u8;
    let mut data = 5u32.to_le_bytes().to_vec();
    data.push(directive);
    stream.write_all(&data)?;
    Ok(())
}

impl ImageReceiver {
    pub fn new() -> Self {
        let core = Arc::new(Mutex::new(ImageReceiverCore::new()));
        Self { core, handle: None }
    }

    pub fn start(&mut self, port: u16) -> Result<(), PbrtError> {
        let hostname = format!("0.0.0.0:{}", port);
        let core = self.core.clone();

        let listener = TcpListener::bind(&hostname)?;
        println!("Starting ImageReceiver on {}", hostname);
        
        let (tx, rx): (Sender<i32>, Receiver<i32>) = mpsc::channel();
        let handle = thread::spawn(move || {
            let core = core.clone();
            loop {
                if rx.try_recv().is_ok() {
                    return;
                }
                //match res {
                match listener.accept() {
                    Ok((mut stream, _addr)) => {
                        println!("Accepted connection from {}", stream.peer_addr().unwrap());
                        loop {
                            if rx.try_recv().is_ok() {
                                return; // Exit the loop if a stop signal is received
                            }

                            match read_bytes(&mut stream) {
                                Ok(data) => {
                                    //println!("Received {} bytes of data", data.len());
                                    if data.is_empty() {
                                        break; // No data to read, exit the loop
                                    }
                                    // Process the received data
                                    match evaluate_bytes(&core, &data) {
                                        Ok(directive) => {
                                            //println!("Received directive: {:?}", directive);
                                            match directive {
                                                DisplayDirective::CloseServer => {
                                                    println!("Received CloseServer directive, stopping receiver.");
                                                    return;
                                                }
                                                _ => {
                                                    // Handle other directives if needed
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            println!("Error evaluating bytes: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!("Error reading from stream: {}", e);
                                    break; // Exit the loop on error
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("Error accepting connection: {}", e);
                        return;
                    }
                }
            }
        });
        println!("ImageReceiver thread started.");
        self.handle = Some((hostname, tx, handle));
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), PbrtError> {
        if let Some((hostname, tx, handle)) = self.handle.take() {
            // Send a signal to stop the thread
            if !handle.is_finished() {
                let _ = tx.send(0);
                send_close_server(&hostname)?;
                handle.join().unwrap();
            }
        }
        Ok(())
    }

    pub fn get_image_data(&self) -> Option<Arc<Mutex<ImageData>>> {
        let core = self.core.lock().unwrap();
        if core.images.is_empty() {
            return None; // No images available
        } else {
            // Return the first image data found
            let first_image = core.images.values().next().cloned();
            return first_image;
        }
    }
}

impl Drop for ImageReceiver {
    fn drop(&mut self) {
        if let Some((hostname, tx, handle)) = self.handle.take() {
            // Send a signal to stop the thread
            if !handle.is_finished() {
                let _ = tx.send(0);
                let _ = send_close_server(&hostname);
                handle.join().unwrap();
            }
        }
    }
}
