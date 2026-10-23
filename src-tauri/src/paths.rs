//! Where Scripta keeps its data: `%LOCALAPPDATA%\io.github.saibot-strg.scripta`.
//!
//! Local, not roaming: with roaming profiles, `%APPDATA%` is copied to a
//! server at logoff, and audio, transcripts and voice profiles must never
//! leave the PC. Not `%LOCALAPPDATA%\Scripta`: the per-user installer puts the
//! program there. The uninstaller removes this folder only if the user ticks
//! "delete the application data". `SCRIPTA_DATA_DIR` overrides it (tests).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SCRIPTA_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("io.github.saibot-strg.scripta")
}

pub fn default_recordings_root() -> PathBuf {
    data_dir().join("Recordings")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("Models")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("Logs")
}

pub fn library_file() -> PathBuf {
    data_dir().join("library.sqlite")
}

pub fn voices_file() -> PathBuf {
    data_dir().join("voices.sqlite")
}

pub fn settings_file() -> PathBuf {
    data_dir().join("settings.json")
}
