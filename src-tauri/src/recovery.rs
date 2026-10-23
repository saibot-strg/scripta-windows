//! Startup maintenance: recovers recordings and imports interrupted by a
//! crash or power loss, removes leftovers of unfinished file operations and
//! finishes pending AAC encodings.

use std::collections::HashSet;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::audio::wav;
use crate::recorder;
use crate::state::{encode_if_needed, tr, AppState};
use crate::storage::{Storage, MANIFEST_FILE};

/// Prefixes and extensions of temporary files that are never complete.
fn is_leftover(name: &str) -> bool {
    ["levelling-", "mixing-", "encoding-", "extracting-"].iter().any(|p| name.starts_with(p))
        || name.ends_with(".part")
        || name.ends_with(".json.tmp")
}

pub fn run(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut roots: HashSet<PathBuf> = HashSet::new();
    roots.insert(crate::paths::default_recordings_root());
    roots.insert(state.current_storage().root);
    if let Ok(stored) = state.library.lock().unwrap().storage_roots() {
        roots.extend(stored.iter().map(|root| Storage::for_root(root.as_deref()).root));
    }

    let active = state.recorder.lock().unwrap().active_id();
    let mut recovered = 0;
    for root in roots {
        state.allow_audio_folder(&root);
        let storage = Storage { root };
        for folder in storage.recording_folders() {
            // Leftovers of interrupted file operations.
            if let Ok(entries) = std::fs::read_dir(&folder) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if is_leftover(&name) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
            if !folder.join(MANIFEST_FILE).exists() {
                continue;
            }
            let Some(manifest) = Storage::read_manifest(&folder) else { continue };
            if active.as_deref() == Some(manifest.id.as_str()) {
                continue;
            }
            if state.library.lock().unwrap().contains(&manifest.id).unwrap_or(false) {
                storage.remove_manifest(&manifest.id);
                continue;
            }
            for name in manifest.captured_file_names() {
                let path = storage.file(&manifest.id, &name);
                if name.ends_with(".wav") && path.exists() {
                    if let Err(error) = wav::repair(&path) {
                        tracing::error!(%error, "repairing track failed");
                    }
                }
            }
            let has_audio = manifest.captured_file_names().iter().any(|name| {
                std::fs::metadata(storage.file(&manifest.id, name)).map(|m| m.len() > 44).unwrap_or(false)
            });
            if !has_audio {
                // Nothing was recorded or copied.
                let _ = storage.remove_folder(&manifest.id);
                continue;
            }
            let levels = state.settings().levels_microphone;
            match recorder::finish(&manifest, &storage, levels).and_then(|duration| state.add_to_library(&manifest, &storage, duration)) {
                Ok(()) => {
                    recovered += 1;
                    tracing::info!(id = %manifest.id, "recording recovered");
                }
                Err(error) => tracing::error!(%error, id = %manifest.id, "recovery failed"),
            }
        }
    }
    if recovered > 0 {
        state.notice(
            tr("Aufnahme wiederhergestellt", "Recording recovered"),
            Some(tr(
                "Scripta wurde während einer Aufnahme beendet. Alles bis zu diesem Zeitpunkt ist gespeichert.",
                "Scripta was closed during a recording. Everything up to that moment is saved.",
            )),
            None,
        );
    }

    // Tracks still waiting for AAC encoding.
    let ids: Vec<String> = state.library.lock().unwrap().summaries().map(|s| s.into_iter().map(|r| r.id).collect()).unwrap_or_default();
    for id in ids {
        encode_if_needed(app, &id);
    }
}
