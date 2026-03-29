use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub folder_name: String,
    pub date: String,
    pub time: String,
    pub duration_secs: Option<u64>,
    pub has_transcript: bool,
    pub transcript_text: Option<String>,
    pub audio_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeetingMeta {
    pub id: String,
    pub date: String,
    pub time: String,
    pub duration_secs: Option<u64>,
    pub speakers: Vec<String>,
    pub tags: Vec<String>,
}

pub fn get_meetings_dir() -> PathBuf {
    let home = dirs::home_dir().expect("Could not find home directory");
    home.join("SoloKeeper").join("meetings")
}

pub fn create_meeting_folder() -> Result<(PathBuf, String), String> {
    let meetings_dir = get_meetings_dir();
    fs::create_dir_all(&meetings_dir)
        .map_err(|e| format!("Failed to create meetings directory: {}", e))?;

    let now = Local::now();
    let folder_name = now.format("%Y-%m-%d_%H-%M_recording").to_string();
    let folder_path = meetings_dir.join(&folder_name);

    fs::create_dir_all(&folder_path)
        .map_err(|e| format!("Failed to create meeting folder: {}", e))?;

    let id = uuid::Uuid::new_v4().to_string();

    let meta = MeetingMeta {
        id: id.clone(),
        date: now.format("%Y-%m-%d").to_string(),
        time: now.format("%H:%M").to_string(),
        duration_secs: None,
        speakers: vec![],
        tags: vec![],
    };

    let meta_path = folder_path.join("meta.json");
    let meta_json =
        serde_json::to_string_pretty(&meta).map_err(|e| format!("Failed to serialize meta: {}", e))?;
    fs::write(&meta_path, meta_json)
        .map_err(|e| format!("Failed to write meta.json: {}", e))?;

    Ok((folder_path, id))
}

pub fn update_meeting_duration(folder_path: &PathBuf, duration_secs: u64) -> Result<(), String> {
    let meta_path = folder_path.join("meta.json");
    let meta_str =
        fs::read_to_string(&meta_path).map_err(|e| format!("Failed to read meta.json: {}", e))?;
    let mut meta: MeetingMeta =
        serde_json::from_str(&meta_str).map_err(|e| format!("Failed to parse meta.json: {}", e))?;

    meta.duration_secs = Some(duration_secs);

    let meta_json =
        serde_json::to_string_pretty(&meta).map_err(|e| format!("Failed to serialize meta: {}", e))?;
    fs::write(&meta_path, meta_json)
        .map_err(|e| format!("Failed to write meta.json: {}", e))?;

    Ok(())
}

pub fn list_recordings() -> Result<Vec<Recording>, String> {
    let meetings_dir = get_meetings_dir();
    if !meetings_dir.exists() {
        return Ok(vec![]);
    }

    let mut recordings = Vec::new();

    let mut entries: Vec<_> = fs::read_dir(&meetings_dir)
        .map_err(|e| format!("Failed to read meetings directory: {}", e))?
        .filter_map(|e| e.ok())
        .collect();

    entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let folder_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let meta_path = path.join("meta.json");
        let audio_path = path.join("audio.wav");
        let transcript_md_path = path.join("transcript.md");

        if !audio_path.exists() && !meta_path.exists() {
            continue;
        }

        let (id, date, time, duration_secs) = if meta_path.exists() {
            match fs::read_to_string(&meta_path) {
                Ok(meta_str) => match serde_json::from_str::<MeetingMeta>(&meta_str) {
                    Ok(meta) => (meta.id, meta.date, meta.time, meta.duration_secs),
                    Err(_) => (
                        folder_name.clone(),
                        folder_name.get(..10).unwrap_or("").to_string(),
                        folder_name.get(11..16).unwrap_or("").to_string(),
                        None,
                    ),
                },
                Err(_) => (
                    folder_name.clone(),
                    folder_name.get(..10).unwrap_or("").to_string(),
                    folder_name.get(11..16).unwrap_or("").to_string(),
                    None,
                ),
            }
        } else {
            (
                folder_name.clone(),
                folder_name.get(..10).unwrap_or("").to_string(),
                folder_name.get(11..16).unwrap_or("").to_string(),
                None,
            )
        };

        let has_transcript = transcript_md_path.exists();
        let transcript_text = if has_transcript {
            fs::read_to_string(&transcript_md_path).ok()
        } else {
            None
        };

        recordings.push(Recording {
            id,
            folder_name,
            date,
            time,
            duration_secs,
            has_transcript,
            transcript_text,
            audio_path: audio_path.to_string_lossy().to_string(),
        });
    }

    Ok(recordings)
}
