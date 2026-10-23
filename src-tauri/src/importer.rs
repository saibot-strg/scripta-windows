//! Brings external audio and video files into the library.
//!
//! Audio files the player can handle are copied unchanged; the user's
//! original stays where it is. Videos and other formats keep only their
//! audio, extracted to AAC. Converting to 16 kHz mono happens later, during
//! transcription.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::mf;
use crate::model::RecordingSource;
use crate::state::{tr, AppState};
use crate::storage::{RecordingManifest, Storage};

/// Formats kept as they are (Windows decodes them, the player plays them).
const COPIED: &[&str] = &["mp3", "m4a", "wav", "flac"];

/// What the file dialog offers.
pub const EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "wav", "flac", "aac", "wma", "ogg", "opus", "aif", "aiff", "caf", "mp4", "m4v", "mov", "mkv", "avi", "wmv",
    "webm", "3gp", "amr",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatus {
    pub total: usize,
    pub completed: usize,
    pub imported_ids: Vec<String>,
    pub is_finished: bool,
    pub is_queued: bool,
}

fn file_label(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Copies or extracts the audio into a new recording folder and writes its
/// manifest. The caller adds it to the library.
pub fn prepare(source: &Path, storage: &Storage, folder_id: Option<String>) -> Result<(RecordingManifest, f64), String> {
    let name = file_label(source);
    let id = uuid::Uuid::new_v4().to_string();
    let extension = source.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let result = (|| -> Result<(RecordingManifest, f64), String> {
        storage.make_folder(&id).map_err(|e| e.to_string())?;
        if !mf::has_audio(source) {
            return Err(tr(&format!("„{name}“: Dieses Format wird nicht unterstützt."), &format!("“{name}”: This format is not supported.")));
        }
        let file_name = if COPIED.contains(&extension.as_str()) {
            let file_name = format!("original.{extension}");
            let destination = storage.file(&id, &file_name);
            // Copy under a temporary name, so a partial copy never looks complete.
            let partial = storage.file(&id, "importing.part");
            std::fs::copy(source, &partial).and_then(|_| std::fs::rename(&partial, &destination)).map_err(|e| {
                tr(&format!("„{name}“ konnte nicht importiert werden: {e}"), &format!("“{name}” could not be imported: {e}"))
            })?;
            file_name
        } else {
            let file_name = "audio.m4a".to_string();
            mf::extract_audio_to_aac(source, &storage.file(&id, &file_name), 128_000).map_err(|e| {
                tr(&format!("„{name}“ konnte nicht importiert werden: {e}"), &format!("“{name}” could not be imported: {e}"))
            })?;
            file_name
        };
        let duration = mf::duration(&storage.file(&id, &file_name)).unwrap_or(0.0);
        if duration <= 0.0 {
            return Err(tr(&format!("„{name}“ enthält keine Tonspur."), &format!("“{name}” has no audio track.")));
        }
        let manifest = RecordingManifest {
            id: id.clone(),
            title: source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name.clone()),
            created_at: chrono::Utc::now(),
            source: RecordingSource::ImportedFile,
            file_name,
            microphone_file_name: None,
            system_file_name: None,
            folder_id,
        };
        // Until it is in the library, an interrupted import is recovered like a recording.
        storage.write_manifest(&manifest).map_err(|e| e.to_string())?;
        Ok((manifest, duration))
    })();
    if result.is_err() {
        let _ = storage.remove_folder(&id);
    }
    result
}

/// Imports files one after another on a background thread; progress goes to
/// the UI as "import-status".
pub fn import(app: &AppHandle, paths: Vec<String>, folder_id: Option<String>) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let storage = state.current_storage();
        let auto = state.settings().auto_transcribe;
        let mut status = ImportStatus { total: paths.len(), completed: 0, imported_ids: Vec::new(), is_finished: false, is_queued: auto };
        let _ = app.emit("import-status", &status);
        let mut failures = Vec::new();
        for path in &paths {
            match prepare(Path::new(path), &storage, folder_id.clone()) {
                Ok((manifest, duration)) => match state.add_to_library(&manifest, &storage, duration) {
                    Ok(()) => status.imported_ids.push(manifest.id.clone()),
                    Err(error) => failures.push(error.to_string()),
                },
                Err(message) => failures.push(message),
            }
            status.completed += 1;
            let _ = app.emit("import-status", &status);
        }
        status.is_finished = true;
        let _ = app.emit("import-status", &status);
        if let Some(last) = status.imported_ids.last() {
            let _ = app.emit("recording-added", last);
        }
        tracing::info!(imported = status.imported_ids.len(), failed = failures.len(), "import finished");
        if !failures.is_empty() {
            state.notice(
                tr("Nicht alle Dateien konnten importiert werden", "Not all files could be imported"),
                Some(failures.join("\n")),
                None,
            );
        }
    });
}
