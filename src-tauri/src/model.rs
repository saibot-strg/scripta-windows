//! Values shared by the library, the transcription pipeline and the UI.

use serde::{Deserialize, Serialize};

/// Where a recording came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingSource {
    Microphone,
    Meeting,
    ImportedFile,
}

impl RecordingSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Meeting => "meeting",
            Self::ImportedFile => "importedFile",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "microphone" => Self::Microphone,
            "meeting" => Self::Meeting,
            _ => Self::ImportedFile,
        }
    }
}

/// Lifecycle of a recording's transcript. Recording and transcription are
/// separate steps, so every recording starts as `NotTranscribed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptionStatus {
    NotTranscribed,
    Queued,
    Running,
    Done,
    Failed,
}

impl TranscriptionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotTranscribed => "notTranscribed",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::NotTranscribed,
        }
    }

    pub fn is_in_progress(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

/// Role of an audio file belonging to a recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackRole {
    /// The file used for playback (mic recording, imported original or meeting mix).
    Primary,
    /// Meeting mode: the local microphone ("Ich").
    Microphone,
    /// Meeting mode: system or app audio ("Andere").
    System,
}

impl TrackRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Microphone => "microphone",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "microphone" => Self::Microphone,
            "system" => Self::System,
            _ => Self::Primary,
        }
    }
}

/// Who is speaking in a transcript segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeakerRole {
    /// Local user, derived from the microphone track.
    Me,
    /// Remote participants, derived from the system audio track.
    Others,
    /// A speaker found by diarization (numbered).
    Diarized,
}

impl SpeakerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Me => "me",
            Self::Others => "others",
            Self::Diarized => "diarized",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "me" => Self::Me,
            "diarized" => Self::Diarized,
            _ => Self::Others,
        }
    }
}

/// Timing of a single word inside a segment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WordTiming {
    pub text: String,
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl WordTiming {
    pub fn new(text: &str, start: f64, end: f64) -> Self {
        Self { text: text.to_string(), start, end, confidence: None }
    }
}

/// UI language for generated names ("Ich" / "Me").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    German,
    English,
}

static LANGUAGE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

impl Language {
    pub fn current() -> Self {
        if LANGUAGE.load(std::sync::atomic::Ordering::Relaxed) == 1 {
            Self::English
        } else {
            Self::German
        }
    }

    pub fn set_current(language: Self) {
        LANGUAGE.store(if language == Self::English { 1 } else { 0 }, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Generated name of a speaker without a custom name.
pub fn default_speaker_name(role: SpeakerRole, number: i64) -> String {
    match (Language::current(), role) {
        (Language::German, SpeakerRole::Me) => "Ich".into(),
        (Language::German, SpeakerRole::Others) => "Andere".into(),
        (Language::German, SpeakerRole::Diarized) => format!("Sprecher {number}"),
        (Language::English, SpeakerRole::Me) => "Me".into(),
        (Language::English, SpeakerRole::Others) => "Others".into(),
        (Language::English, SpeakerRole::Diarized) => format!("Speaker {number}"),
    }
}
