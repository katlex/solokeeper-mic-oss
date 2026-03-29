mod audio;
mod recordings;
mod settings;

use audio::AudioRecorder;
use recordings::{Recording, create_meeting_folder, list_recordings, update_meeting_duration, update_speaker_names, search_recordings};
use settings::{AppSettings, load_settings, save_settings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{Manager, menu::{MenuBuilder, MenuItemBuilder}, tray::TrayIconBuilder};

struct AppState {
    recorder: Mutex<AudioRecorder>,
    current_folder: Mutex<Option<PathBuf>>,
    recording_start: Mutex<Option<Instant>>,
}

#[tauri::command]
fn get_input_devices() -> Vec<String> {
    AudioRecorder::get_input_devices()
}

#[tauri::command]
fn start_recording(state: tauri::State<AppState>) -> Result<String, String> {
    let (folder_path, id) = create_meeting_folder()?;
    let audio_path = folder_path.join("audio.wav");

    let recorder = state.recorder.lock().unwrap();
    recorder.start_recording(audio_path)?;

    *state.current_folder.lock().unwrap() = Some(folder_path);
    *state.recording_start.lock().unwrap() = Some(Instant::now());

    Ok(id)
}

#[tauri::command]
fn stop_recording(state: tauri::State<AppState>) -> Result<String, String> {
    let recorder = state.recorder.lock().unwrap();
    recorder.stop_recording()?;

    let folder_path = state.current_folder.lock().unwrap().clone();
    let start_time = state.recording_start.lock().unwrap().take();

    if let (Some(folder), Some(start)) = (&folder_path, start_time) {
        let duration = start.elapsed().as_secs();
        update_meeting_duration(folder, duration)?;
    }

    let path = folder_path
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    *state.current_folder.lock().unwrap() = None;

    Ok(path)
}

#[tauri::command]
fn get_recording_status(state: tauri::State<AppState>) -> (bool, f32, u64) {
    let recorder = state.recorder.lock().unwrap();
    let is_recording = recorder.is_recording();
    let level = recorder.get_level();
    let elapsed = state
        .recording_start
        .lock()
        .unwrap()
        .map(|s| s.elapsed().as_secs())
        .unwrap_or(0);
    (is_recording, level, elapsed)
}

#[tauri::command]
fn get_recordings() -> Result<Vec<Recording>, String> {
    list_recordings()
}

#[tauri::command]
fn search_transcripts(query: String) -> Result<Vec<Recording>, String> {
    search_recordings(&query)
}

#[tauri::command]
async fn transcribe_recording(folder_path: String, language: Option<String>) -> Result<String, String> {
    let audio_path = PathBuf::from(&folder_path).join("audio.wav");
    if !audio_path.exists() {
        return Err("Audio file not found".to_string());
    }

    let settings = load_settings();

    // Push config to sidecar (model + HF token)
    let client = reqwest::Client::new();
    let _ = client
        .post("http://127.0.0.1:8384/config")
        .json(&serde_json::json!({
            "whisper_model": settings.whisper_model,
            "hf_token": settings.hf_token,
        }))
        .send()
        .await;

    let lang = match language {
        Some(l) if l != "auto" && !l.is_empty() => Some(l),
        _ => {
            let s = &settings.language;
            if s != "auto" && !s.is_empty() { Some(s.clone()) } else { None }
        }
    };

    let mut body = serde_json::json!({
        "audio_path": audio_path.to_string_lossy(),
        "output_dir": folder_path,
        "diarize": !settings.hf_token.is_empty(),
    });
    if let Some(l) = lang {
        body["language"] = serde_json::Value::String(l);
    }

    let response = client
        .post("http://127.0.0.1:8384/transcribe")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to connect to transcription service: {}", e))?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("Transcription failed: {}", error_text));
    }

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse transcription result: {}", e))?;

    Ok(result.to_string())
}

#[tauri::command]
async fn check_sidecar_status() -> Result<bool, String> {
    let client = reqwest::Client::new();
    match client.get("http://127.0.0.1:8384/health").send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
fn get_settings() -> AppSettings {
    load_settings()
}

#[tauri::command]
fn save_app_settings(settings: AppSettings) -> Result<(), String> {
    save_settings(&settings)
}

#[tauri::command]
async fn save_speaker_names(folder_path: String, speaker_names: HashMap<String, String>) -> Result<(), String> {
    update_speaker_names(&folder_path, speaker_names)?;

    // Ask sidecar to regenerate transcript.md
    let client = reqwest::Client::new();
    let _ = client
        .post("http://127.0.0.1:8384/regenerate-md")
        .json(&serde_json::json!({ "output_dir": folder_path }))
        .send()
        .await;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            recorder: Mutex::new(AudioRecorder::new()),
            current_folder: Mutex::new(None),
            recording_start: Mutex::new(None),
        })
        .setup(|app| {
            // Set up tray icon with menu
            let quit = MenuItemBuilder::new("Quit").id("quit").build(app)?;
            let show = MenuItemBuilder::new("Show Window").id("show").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&show)
                .separator()
                .item(&quit)
                .build()?;

            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("SoloKeeper Mic")
                .on_menu_event(move |app, event| {
                    match event.id().as_ref() {
                        "quit" => {
                            app.exit(0);
                        }
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_input_devices,
            start_recording,
            stop_recording,
            get_recording_status,
            get_recordings,
            search_transcripts,
            transcribe_recording,
            check_sidecar_status,
            get_settings,
            save_app_settings,
            save_speaker_names,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
