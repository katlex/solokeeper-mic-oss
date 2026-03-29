mod audio;
mod recordings;
mod settings;

use audio::{AudioRecorder, merge_to_audio_wav};
use recordings::{Recording, create_meeting_folder, list_recordings, update_meeting_duration, update_speaker_names, search_recordings};
use settings::{AppSettings, load_settings, save_settings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{Manager, menu::{MenuBuilder, MenuItemBuilder}, tray::TrayIconBuilder};

struct AppState {
    mic_recorder: Mutex<AudioRecorder>,
    system_recorder: Mutex<AudioRecorder>,
    current_folder: Mutex<Option<PathBuf>>,
    recording_start: Mutex<Option<Instant>>,
}

#[tauri::command]
fn get_input_devices() -> Vec<String> {
    AudioRecorder::get_input_devices()
}

#[tauri::command]
fn start_recording(state: tauri::State<AppState>) -> Result<String, String> {
    let settings = load_settings();
    let (folder_path, id) = create_meeting_folder()?;

    // Start mic recording
    let mic_path = if settings.system_audio_device.is_some() {
        folder_path.join("mic.wav")
    } else {
        folder_path.join("audio.wav")
    };

    let mic_recorder = state.mic_recorder.lock().unwrap();
    mic_recorder.start_recording(mic_path, settings.audio_input_device.as_deref())?;

    // Start system audio recording if a device is configured
    if let Some(ref sys_device) = settings.system_audio_device {
        let sys_path = folder_path.join("system.wav");
        let sys_recorder = state.system_recorder.lock().unwrap();
        if let Err(e) = sys_recorder.start_recording(sys_path, Some(sys_device.as_str())) {
            eprintln!("Failed to start system audio capture: {}. Continuing with mic only.", e);
        }
    }

    *state.current_folder.lock().unwrap() = Some(folder_path);
    *state.recording_start.lock().unwrap() = Some(Instant::now());

    Ok(id)
}

#[tauri::command]
fn stop_recording(state: tauri::State<AppState>) -> Result<String, String> {
    // Stop mic
    let mic_recorder = state.mic_recorder.lock().unwrap();
    mic_recorder.stop_recording()?;

    // Stop system audio if it was recording
    let sys_recorder = state.system_recorder.lock().unwrap();
    if sys_recorder.is_recording() {
        sys_recorder.stop_recording()?;
    }

    let folder_path = state.current_folder.lock().unwrap().clone();
    let start_time = state.recording_start.lock().unwrap().take();

    if let (Some(folder), Some(start)) = (&folder_path, start_time) {
        let duration = start.elapsed().as_secs();
        update_meeting_duration(folder, duration)?;
    }

    // Merge mic + system into stereo audio.wav if both exist
    if let Some(ref folder) = folder_path {
        let mic_path = folder.join("mic.wav");
        if mic_path.exists() {
            merge_to_audio_wav(folder)?;
        }
    }

    let path = folder_path
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    *state.current_folder.lock().unwrap() = None;

    Ok(path)
}

#[tauri::command]
fn get_recording_status(state: tauri::State<AppState>) -> (bool, f32, u64) {
    let recorder = state.mic_recorder.lock().unwrap();
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
fn get_audio_base64(path: String) -> Result<String, String> {
    use base64::Engine;
    let bytes = std::fs::read(&path).map_err(|e| format!("Failed to read audio: {}", e))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
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
            mic_recorder: Mutex::new(AudioRecorder::new()),
            system_recorder: Mutex::new(AudioRecorder::new()),
            current_folder: Mutex::new(None),
            recording_start: Mutex::new(None),
        })
        .register_uri_scheme_protocol("audiofile", |_app, request| {
            let uri = request.uri();
            let path_str = uri.path();
            // Strip leading slash, then percent-decode
            let trimmed = path_str.strip_prefix('/').unwrap_or(path_str);
            let decoded = percent_decode(trimmed);
            eprintln!("[audiofile] Requested: {} -> {}", path_str, decoded);

            match std::fs::read(&decoded) {
                Ok(content) => {
                    let len = content.len();
                    tauri::http::Response::builder()
                        .header("Content-Type", "audio/wav")
                        .header("Content-Length", len.to_string())
                        .header("Accept-Ranges", "bytes")
                        .header("Access-Control-Allow-Origin", "*")
                        .body(content)
                        .unwrap()
                }
                Err(_) => {
                    tauri::http::Response::builder()
                        .status(404)
                        .body(b"File not found".to_vec())
                        .unwrap()
                }
            }
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
            get_audio_base64,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Simple percent-decoding for URI paths.
fn percent_decode(input: &str) -> String {
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(
                &String::from_utf8_lossy(&bytes[i + 1..i + 3]),
                16,
            ) {
                result.push(val);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}
