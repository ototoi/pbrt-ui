use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::PbrtError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub pbrt_executable_path: PathBuf,
    pub enable_display_server: bool,
    pub display_server_host: String,
    pub display_server_port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pbrt_executable_path: PathBuf::from(""),
            enable_display_server: false,
            display_server_host: "localhost".to_string(),
            display_server_port: 14158,
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
