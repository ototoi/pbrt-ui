use crate::error::PbrtError;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub pbrt_executable_path: PathBuf,
    pub enable_display_server: bool,
    pub display_server_host: String,
    pub display_server_port: u16,
    pub render_output_directory: String,
    pub import_file_directory: String,
    pub export_file_directory: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut render_output_directory: String = "".to_string();
        if let Some(path) = dirs::document_dir() {
            let path = path.join("pbrt_ui").to_str().unwrap().to_string();
            render_output_directory = path;
        }
        let mut import_file_directory: String = "".to_string();
        if let Some(path) = dirs::document_dir() {
            let path = path.join("pbrt_ui").to_str().unwrap().to_string();
            import_file_directory = path;
        }
        let mut export_file_directory: String = "".to_string();
        if let Some(path) = dirs::document_dir() {
            let path = path.join("pbrt_ui").to_str().unwrap().to_string();
            export_file_directory = path;
        }

        Self {
            pbrt_executable_path: PathBuf::from(""),
            enable_display_server: false,
            display_server_host: "localhost".to_string(),
            display_server_port: 14158,
            render_output_directory: render_output_directory,
            import_file_directory: import_file_directory,
            export_file_directory: export_file_directory,
        }
    }
}

impl AppConfig {
    pub fn load_from_file(path: &PathBuf) -> Result<Self, PbrtError> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        match serde_json::from_reader(reader) {
            Ok(config) => Ok(config),
            Err(e) => Err(PbrtError::error(&format!("{}", e))),
        }
    }
}
