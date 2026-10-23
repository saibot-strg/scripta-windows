//! App-wide state shared by the commands and the background threads.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::{mf, wav};
use crate::library::{Library, NewRecording};
use crate::model::{Language, RecordingSource, TrackRole, TranscriptionStatus};
use crate::recorder::Recorder;
use crate::service::TranscriptionService;
use crate::settings::{Settings, SettingsStore};
use crate::storage::{RecordingManifest, Storage};
use crate::voices::VoiceStore;

/// German or English text, by the UI language.
pub fn tr(german: &str, english: &str) -> String {
    match Language::current() {
        Language::German => german.to_string(),
        Language::English => english.to_string(),
    }
}

pub struct AppState {
    pub app: AppHandle,
    pub library: Mutex<Library>,
    pub voices: Mutex<VoiceStore>,
    settings: Mutex<SettingsStore>,
    pub service: TranscriptionService,
    pub recorder: Mutex<Recorder>,
    /// Recordings whose tracks are being encoded to AAC.
    encoding: Mutex<HashSet<String>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub title: String,
    pub message: Option<String>,
    /// "microphone": offer to open the Windows privacy settings.
    pub action: Option<String>,
}

impl AppState {
    pub fn new(app: AppHandle, library: Library, voices: VoiceStore, settings: SettingsStore) -> Self {
        Self {
            service: TranscriptionService::start(app.clone()),
            app,
            library: Mutex::new(library),
            voices: Mutex::new(voices),
            settings: Mutex::new(settings),
            recorder: Mutex::new(Recorder::default()),
            encoding: Mutex::new(HashSet::new()),
        }
    }

    // MARK: Settings

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().get().clone()
    }

    pub fn update_settings(&self, patch: &serde_json::Value) -> Result<Settings> {
        let settings = self.settings.lock().unwrap().update(patch)?;
        let _ = self.app.emit("settings-changed", &settings);
        Ok(settings)
    }

    pub fn modify_settings(&self, change: impl FnOnce(&mut Settings)) {
        let result = {
            let mut store = self.settings.lock().unwrap();
            store.modify(change).map(|_| store.get().clone())
        };
        match result {
            Ok(settings) => {
                let _ = self.app.emit("settings-changed", &settings);
            }
            Err(error) => tracing::error!(%error, "saving settings failed"),
        }
    }

    /// Where new recordings go.
    pub fn current_storage(&self) -> Storage {
        Storage::for_root(self.settings().storage_root.as_deref())
    }

    /// The storage root to store with a recording (None = the default).
    pub fn root_for_library(storage: &Storage) -> Option<String> {
        (storage.root != crate::paths::default_recordings_root()).then(|| storage.root.to_string_lossy().into_owned())
    }

    /// Lets the web view play files below the folder.
    pub fn allow_audio_folder(&self, folder: &Path) {
        if let Err(error) = self.app.asset_protocol_scope().allow_directory(folder, true) {
            tracing::error!(%error, "asset scope");
        }
    }

    // MARK: Events

    /// The library changed; `id` is the recording to reload if it is shown.
    pub fn notify_library(&self, id: Option<&str>) {
        let _ = self.app.emit("library-changed", serde_json::json!({ "id": id }));
    }

    pub fn notice(&self, title: String, message: Option<String>, action: Option<&str>) {
        let _ = self.app.emit("notice", Notice { title, message, action: action.map(str::to_string) });
    }

    pub fn toast(&self, message: String) {
        let _ = self.app.emit("toast", message);
    }

    pub fn set_status(&self, id: &str, status: TranscriptionStatus, error: Option<&str>) {
        if let Err(error) = self.library.lock().unwrap().set_status(id, status, error) {
            tracing::error!(%error, "saving status failed");
        }
        self.notify_library(Some(id));
    }

    // MARK: Transcription

    pub fn enqueue(&self, id: &str, speaker_count: Option<usize>) {
        if self.service.is_queued(id) {
            return;
        }
        self.set_status(id, TranscriptionStatus::Queued, None);
        self.service.enqueue(id, speaker_count);
    }

    pub fn cancel(&self, id: &str) {
        if self.service.cancel(id) {
            let has_transcript = self.library.lock().unwrap().has_transcript(id).unwrap_or(false);
            let status = if has_transcript { TranscriptionStatus::Done } else { TranscriptionStatus::NotTranscribed };
            self.set_status(id, status, None);
        }
    }

    pub fn is_encoding(&self, id: &str) -> bool {
        self.encoding.lock().unwrap().contains(id)
    }

    // MARK: New recordings

    /// Adds a finished recording or import to the library, then encodes and
    /// transcribes it in the background as the settings say.
    pub fn add_to_library(&self, manifest: &RecordingManifest, storage: &Storage, duration: f64) -> Result<()> {
        let tracks = if manifest.is_meeting() {
            let mut tracks = vec![(TrackRole::Primary, manifest.file_name.clone(), duration)];
            for (role, name) in [(TrackRole::Microphone, &manifest.microphone_file_name), (TrackRole::System, &manifest.system_file_name)] {
                if let Some(name) = name {
                    let length = wav::duration(&storage.file(&manifest.id, name)).unwrap_or(duration);
                    tracks.push((role, name.clone(), length));
                }
            }
            tracks
        } else {
            vec![(TrackRole::Primary, manifest.file_name.clone(), duration)]
        };
        self.library.lock().unwrap().add_recording(&NewRecording {
            id: manifest.id.clone(),
            title: manifest.title.clone(),
            created_at: manifest.created_at,
            duration,
            source: manifest.source,
            storage_root: Self::root_for_library(storage),
            tracks,
            folder_id: manifest.folder_id.clone(),
        })?;
        storage.remove_manifest(&manifest.id);
        self.notify_library(Some(&manifest.id));
        tracing::info!(id = %manifest.id, source = manifest.source.as_str(), duration, "added to library");
        if manifest.source != RecordingSource::ImportedFile {
            encode_if_needed(&self.app, &manifest.id);
        }
        if self.settings().auto_transcribe {
            self.enqueue(&manifest.id, None);
        }
        Ok(())
    }
}

/// Encodes the WAV tracks of a recording to AAC in the background when AAC
/// is selected. Until then (or if encoding fails) the WAV files stay usable.
pub fn encode_if_needed(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let settings = state.settings();
    if !settings.encodes_aac() {
        return;
    }
    let (storage, pending) = {
        let library = state.library.lock().unwrap();
        let Ok(Some(recording)) = library.recording(id) else { return };
        let tracks = library.tracks(id).unwrap_or_default();
        let pending: Vec<_> = tracks.into_iter().filter(|t| t.file_name.ends_with(".wav")).collect();
        (Storage::for_root(recording.storage_root.as_deref()), pending)
    };
    if pending.is_empty() || !state.encoding.lock().unwrap().insert(id.to_string()) {
        return;
    }
    let app = app.clone();
    let id = id.to_string();
    let bit_rate = settings.aac_bit_rate();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        for track in pending {
            let source = storage.file(&id, &track.file_name);
            let encoded = track.file_name.trim_end_matches(".wav").to_string() + ".m4a";
            let destination = storage.file(&id, &encoded);
            match mf::encode_wav_to_aac(&source, &destination, bit_rate) {
                Ok(_) => {
                    // One lock at a time: a guard in the first operand lives to the end of the statement.
                    let replaced = state.library.lock().unwrap().replace_track_file(&track.id, &encoded).is_ok();
                    let still_there = replaced && state.library.lock().unwrap().contains(&id).unwrap_or(false);
                    if still_there {
                        if let Err(error) = std::fs::remove_file(&source) {
                            tracing::warn!(%error, "WAV kept after encoding");
                        }
                    } else {
                        let _ = std::fs::remove_file(&destination);
                    }
                }
                Err(error) => tracing::error!(%error, file = %track.file_name, "AAC encoding failed"),
            }
        }
        state.encoding.lock().unwrap().remove(&id);
        state.notify_library(Some(&id));
    });
}
