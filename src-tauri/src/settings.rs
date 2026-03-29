use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub audio_input_device: Option<String>,
    pub whisper_model: String,
    pub language: String,
    pub storage_path: String,
    pub recording_mode: String,
    pub hf_token: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        let default_storage = dirs::home_dir()
            .unwrap_or_default()
            .join("SoloKeeper")
            .join("meetings")
            .to_string_lossy()
            .to_string();

        Self {
            audio_input_device: None,
            whisper_model: "small".to_string(),
            language: "auto".to_string(),
            storage_path: default_storage,
            recording_mode: "manual".to_string(),
            hf_token: String::new(),
        }
    }
}

fn settings_path() -> PathBuf {
    let home = dirs::home_dir().expect("Could not find home directory");
    let config_dir = home.join("SoloKeeper");
    fs::create_dir_all(&config_dir).ok();
    config_dir.join("settings.json")
}

pub fn load_settings() -> AppSettings {
    let path = settings_path();
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(settings) => return settings,
                Err(e) => eprintln!("Failed to parse settings: {}", e),
            },
            Err(e) => eprintln!("Failed to read settings: {}", e),
        }
    }
    AppSettings::default()
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path();
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write settings: {}", e))?;
    Ok(())
}
