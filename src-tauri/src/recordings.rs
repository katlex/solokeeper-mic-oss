use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub folder_name: String,
    pub folder_path: String,
    pub date: String,
    pub time: String,
    pub duration_secs: Option<u64>,
    pub has_transcript: bool,
    pub transcript_text: Option<String>,
    pub transcript_json: Option<serde_json::Value>,
    pub audio_path: String,
    pub speakers: Vec<String>,
    pub speaker_names: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeetingMeta {
    pub id: String,
    pub date: String,
    pub time: String,
    pub duration_secs: Option<u64>,
    pub speakers: Vec<String>,
    pub tags: Vec<String>,
    #[serde(default)]
    pub speaker_names: HashMap<String, String>,
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
        speaker_names: HashMap::new(),
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

pub fn update_speaker_names(folder_path: &str, speaker_names: HashMap<String, String>) -> Result<(), String> {
    let path = PathBuf::from(folder_path);
    let meta_path = path.join("meta.json");
    let meta_str =
        fs::read_to_string(&meta_path).map_err(|e| format!("Failed to read meta.json: {}", e))?;
    let mut meta: MeetingMeta =
        serde_json::from_str(&meta_str).map_err(|e| format!("Failed to parse meta.json: {}", e))?;

    meta.speaker_names = speaker_names;

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

        if let Some(rec) = load_recording(&path) {
            recordings.push(rec);
        }
    }

    Ok(recordings)
}

fn load_recording(path: &Path) -> Option<Recording> {
    let folder_name = path.file_name()?.to_string_lossy().to_string();
    let meta_path = path.join("meta.json");
    let audio_path = path.join("audio.wav");
    let transcript_md_path = path.join("transcript.md");
    let transcript_json_path = path.join("transcript.json");

    if !audio_path.exists() && !meta_path.exists() {
        return None;
    }

    let (id, date, time, duration_secs, speakers, speaker_names) = if meta_path.exists() {
        match fs::read_to_string(&meta_path) {
            Ok(meta_str) => match serde_json::from_str::<MeetingMeta>(&meta_str) {
                Ok(meta) => (
                    meta.id,
                    meta.date,
                    meta.time,
                    meta.duration_secs,
                    meta.speakers,
                    meta.speaker_names,
                ),
                Err(_) => default_from_folder(&folder_name),
            },
            Err(_) => default_from_folder(&folder_name),
        }
    } else {
        default_from_folder(&folder_name)
    };

    let has_transcript = transcript_md_path.exists();
    let transcript_text = if has_transcript {
        fs::read_to_string(&transcript_md_path).ok()
    } else {
        None
    };
    let transcript_json = if transcript_json_path.exists() {
        fs::read_to_string(&transcript_json_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    Some(Recording {
        id,
        folder_name,
        folder_path: path.to_string_lossy().to_string(),
        date,
        time,
        duration_secs,
        has_transcript,
        transcript_text,
        transcript_json,
        audio_path: audio_path.to_string_lossy().to_string(),
        speakers,
        speaker_names,
    })
}

fn default_from_folder(folder_name: &str) -> (String, String, String, Option<u64>, Vec<String>, HashMap<String, String>) {
    (
        folder_name.to_string(),
        folder_name.get(..10).unwrap_or("").to_string(),
        folder_name.get(11..16).unwrap_or("").to_string(),
        None,
        vec![],
        HashMap::new(),
    )
}

pub fn search_recordings(query: &str) -> Result<Vec<Recording>, String> {
    let all = list_recordings()?;
    let query_lower = query.to_lowercase();

    let results: Vec<Recording> = all
        .into_iter()
        .filter(|rec| {
            // Search in transcript text
            if let Some(ref text) = rec.transcript_text {
                if text.to_lowercase().contains(&query_lower) {
                    return true;
                }
            }
            // Search in speaker names
            for name in rec.speaker_names.values() {
                if name.to_lowercase().contains(&query_lower) {
                    return true;
                }
            }
            // Search in date
            if rec.date.contains(&query_lower) {
                return true;
            }
            false
        })
        .collect();

    Ok(results)
}
