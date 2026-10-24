//! User settings, stored as JSON next to the library.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// New recordings and imports go straight into the queue (off: recording
    /// and transcribing are separate steps).
    pub auto_transcribe: bool,
    /// "system", "light" or "dark".
    pub appearance: String,
    /// "aac" or "lossless".
    pub recording_format: String,
    /// "standard" (96 kbit/s) or "high" (160 kbit/s).
    pub recording_quality: String,
    /// Endpoint ID of the microphone ("" = system default).
    pub microphone: String,
    /// Raise quiet microphone recordings to a normal speech level.
    pub levels_microphone: bool,
    /// Executable recorded in meetings by default ("" = all system audio).
    pub meeting_source: String,
    /// Tell speakers apart by their voices when transcribing.
    pub separates_speakers: bool,
    /// Meetings: also split the microphone track (several people in the room).
    pub separates_room_speakers: bool,
    /// Remember voices and recognize people in later recordings (opt-in:
    /// voice profiles are biometric data).
    pub recognizes_voices: bool,
    /// Run speech recognition on the graphics card (DirectML) instead of the
    /// processor.
    pub uses_gpu: bool,
    /// The graphics card by name ("" = the one with the most memory).
    pub gpu_name: String,
    /// Scripta ended during work on the graphics card, so it was turned off
    /// again (shown in the settings until the user turns it on again).
    pub gpu_failed: bool,
    /// Look for a new release on GitHub once a day.
    pub checks_for_updates: bool,
    pub last_update_check: Option<String>,
    /// Latest release seen on GitHub, so the update note survives a relaunch.
    pub latest_release_version: Option<String>,
    pub latest_release_page: Option<String>,
    /// Release the user chose to skip, e.g. "0.2.0".
    pub skipped_update_version: Option<String>,
    /// Folder for new recordings (None = the default under the app data).
    pub storage_root: Option<String>,
    /// Show the icon in the notification area.
    pub shows_tray_icon: bool,
    /// The welcome screen was read.
    pub welcome_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_transcribe: false,
            appearance: "system".into(),
            recording_format: "aac".into(),
            recording_quality: "standard".into(),
            microphone: String::new(),
            levels_microphone: true,
            meeting_source: String::new(),
            separates_speakers: true,
            separates_room_speakers: false,
            recognizes_voices: false,
            uses_gpu: false,
            gpu_name: String::new(),
            gpu_failed: false,
            checks_for_updates: true,
            last_update_check: None,
            latest_release_version: None,
            latest_release_page: None,
            skipped_update_version: None,
            storage_root: None,
            shows_tray_icon: true,
            welcome_done: false,
        }
    }
}

impl Settings {
    /// AAC bit rate for the chosen quality (the Windows encoder offers 96,
    /// 128, 160 and 192 kbit/s).
    pub fn aac_bit_rate(&self) -> u32 {
        if self.recording_quality == "high" {
            160_000
        } else {
            96_000
        }
    }

    pub fn encodes_aac(&self) -> bool {
        self.recording_format != "lossless"
    }
}

pub struct SettingsStore {
    path: PathBuf,
    current: Settings,
}

impl SettingsStore {
    pub fn load(path: &Path) -> Self {
        let current = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path: path.to_path_buf(), current }
    }

    pub fn get(&self) -> &Settings {
        &self.current
    }

    /// Applies the fields of `patch` (a partial settings object) and saves.
    pub fn update(&mut self, patch: &serde_json::Value) -> Result<Settings> {
        let mut value = serde_json::to_value(&self.current)?;
        if let (Some(target), Some(source)) = (value.as_object_mut(), patch.as_object()) {
            for (key, field) in source {
                if target.contains_key(key) {
                    target.insert(key.clone(), field.clone());
                }
            }
        }
        self.current = serde_json::from_value(value)?;
        self.save()?;
        Ok(self.current.clone())
    }

    pub fn modify(&mut self, change: impl FnOnce(&mut Settings)) -> Result<()> {
        change(&mut self.current);
        self.save()
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self.current)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_are_applied_and_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut store = SettingsStore::load(&path);
        assert!(store.get().separates_speakers);
        store.update(&serde_json::json!({ "recognizesVoices": true, "unknownKey": 1 })).unwrap();
        let reloaded = SettingsStore::load(&path);
        assert!(reloaded.get().recognizes_voices);
        assert_eq!(reloaded.get().appearance, "system");
    }
}
