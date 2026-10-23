//! Recording folders on disk and the manifest written next to an
//! in-progress recording.
//!
//! The manifest is removed once the recording is in the library. A manifest
//! found at launch means the app ended unexpectedly while recording or
//! importing; the audio written so far is then added to the library.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::RecordingSource;
use crate::paths;

pub const MANIFEST_FILE: &str = "recording.json";
pub const RECORDING_FILE: &str = "recording.wav";
pub const MICROPHONE_FILE: &str = "microphone.wav";
pub const SYSTEM_FILE: &str = "system.wav";
pub const MIX_FILE: &str = "mix.wav";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingManifest {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub source: RecordingSource,
    /// Playback file inside the recording folder. For meetings it is the mix
    /// of both tracks, created after recording stops.
    pub file_name: String,
    /// Meeting only: the separate microphone ("Ich") and system ("Andere") tracks.
    pub microphone_file_name: Option<String>,
    pub system_file_name: Option<String>,
    /// Imports dropped onto a folder land there.
    #[serde(default)]
    pub folder_id: Option<String>,
}

impl RecordingManifest {
    pub fn is_meeting(&self) -> bool {
        self.system_file_name.is_some()
    }

    /// Track files written while recording.
    pub fn captured_file_names(&self) -> Vec<String> {
        if self.is_meeting() {
            [self.microphone_file_name.clone(), self.system_file_name.clone()].into_iter().flatten().collect()
        } else {
            vec![self.file_name.clone()]
        }
    }

    /// The microphone ("Ich") track; a microphone recording has no other.
    pub fn microphone_track(&self) -> Option<String> {
        match self.source {
            RecordingSource::Meeting => self.microphone_file_name.clone(),
            RecordingSource::Microphone => Some(self.file_name.clone()),
            RecordingSource::ImportedFile => None,
        }
    }
}

/// A folder holding one folder per recording.
#[derive(Clone, Debug, PartialEq)]
pub struct Storage {
    pub root: PathBuf,
}

impl Storage {
    /// The root stored with a recording (None = the default).
    pub fn for_root(root: Option<&str>) -> Self {
        Self { root: root.map(PathBuf::from).unwrap_or_else(paths::default_recordings_root) }
    }

    pub fn folder(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn file(&self, id: &str, name: &str) -> PathBuf {
        self.folder(id).join(name)
    }

    pub fn make_folder(&self, id: &str) -> Result<PathBuf> {
        let folder = self.folder(id);
        std::fs::create_dir_all(&folder)?;
        Ok(folder)
    }

    pub fn remove_folder(&self, id: &str) -> Result<()> {
        let folder = self.folder(id);
        if folder.exists() {
            std::fs::remove_dir_all(folder)?;
        }
        Ok(())
    }

    pub fn write_manifest(&self, manifest: &RecordingManifest) -> Result<()> {
        let path = self.file(&manifest.id, MANIFEST_FILE);
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(manifest)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }

    pub fn read_manifest(folder: &Path) -> Option<RecordingManifest> {
        let text = std::fs::read_to_string(folder.join(MANIFEST_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn remove_manifest(&self, id: &str) {
        let _ = std::fs::remove_file(self.file(id, MANIFEST_FILE));
    }

    /// All recording folders below the root.
    pub fn recording_folders(&self) -> Vec<PathBuf> {
        std::fs::read_dir(&self.root)
            .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default()
    }

    /// Free bytes on the drive of the root, if known.
    pub fn available_space(&self) -> Option<u64> {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut probe = self.root.clone();
        while !probe.exists() {
            probe = probe.parent()?.to_path_buf();
        }
        let mut available = 0u64;
        unsafe {
            GetDiskFreeSpaceExW(&HSTRING::from(probe.as_os_str()), Some(&mut available), None, None).ok()?;
        }
        Some(available)
    }
}

/// About an hour of a meeting in lossless PCM, before encoding.
pub const MINIMUM_FREE_SPACE: u64 = 1_000_000_000;
