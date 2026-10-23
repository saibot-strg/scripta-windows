//! The library database: recordings, their audio tracks, transcripts,
//! speakers and folders. Audio never lives in the database; each recording
//! owns a folder named after its ID inside a storage root.
//!
//! Schema changes are numbered migrations (`PRAGMA user_version`); never
//! change a released migration, add a new one.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::export::{ExportDocument, ExportSegment};
use crate::model::{default_speaker_name, RecordingSource, SpeakerRole, TrackRole, TranscriptionStatus, WordTiming};
use crate::transcript::{DraftTranscript, SpeakerKey};

const MIGRATIONS: &[&str] = &[
    // 1: initial schema.
    "CREATE TABLE folders (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE TABLE recordings (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        created_at TEXT NOT NULL,
        duration REAL NOT NULL DEFAULT 0,
        source TEXT NOT NULL,
        status TEXT NOT NULL,
        error_message TEXT,
        language TEXT,
        storage_root TEXT,
        folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL
    );
    CREATE TABLE tracks (
        id TEXT PRIMARY KEY,
        recording_id TEXT NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
        role TEXT NOT NULL,
        file_name TEXT NOT NULL,
        duration REAL NOT NULL DEFAULT 0
    );
    CREATE INDEX tracks_recording ON tracks(recording_id);
    CREATE TABLE transcripts (
        id TEXT PRIMARY KEY,
        recording_id TEXT NOT NULL UNIQUE REFERENCES recordings(id) ON DELETE CASCADE,
        created_at TEXT NOT NULL,
        modified_at TEXT NOT NULL,
        language TEXT,
        engine TEXT NOT NULL,
        full_text TEXT NOT NULL DEFAULT '',
        search_text TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE speakers (
        id TEXT PRIMARY KEY,
        transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
        role TEXT NOT NULL,
        number INTEGER NOT NULL DEFAULT 0,
        custom_name TEXT
    );
    CREATE INDEX speakers_transcript ON speakers(transcript_id);
    CREATE TABLE segments (
        id TEXT PRIMARY KEY,
        transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
        idx INTEGER NOT NULL,
        start REAL NOT NULL,
        end REAL NOT NULL,
        text TEXT NOT NULL,
        original_text TEXT,
        words TEXT,
        speaker_id TEXT REFERENCES speakers(id) ON DELETE SET NULL
    );
    CREATE INDEX segments_transcript ON segments(transcript_id);
    CREATE INDEX segments_speaker ON segments(speaker_id);",
];

/// Case- and diacritic-insensitive form for searching.
pub fn search_key(text: &str) -> String {
    text.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub duration: f64,
    pub source: RecordingSource,
    pub status: TranscriptionStatus,
    pub error_message: Option<String>,
    pub language: Option<String>,
    pub folder_id: Option<String>,
    pub has_transcript: bool,
    /// Filled in by the caller (needs the file system).
    pub has_audio: bool,
}

#[derive(Clone, Debug)]
pub struct RecordingRow {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub duration: f64,
    pub source: RecordingSource,
    pub status: TranscriptionStatus,
    pub storage_root: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackRow {
    pub id: String,
    pub role: TrackRole,
    pub file_name: String,
    pub duration: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerView {
    pub id: String,
    pub role: SpeakerRole,
    pub number: i64,
    pub custom_name: Option<String>,
    pub display_name: String,
    pub default_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentView {
    pub id: String,
    pub index: i64,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub is_edited: bool,
    pub speaker_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptView {
    pub id: String,
    pub language: Option<String>,
    pub engine: String,
    pub modified_at: String,
    pub speakers: Vec<SpeakerView>,
    pub segments: Vec<SegmentView>,
}

/// A new recording for the library (from a recording, an import or recovery).
pub struct NewRecording {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub duration: f64,
    pub source: RecordingSource,
    pub storage_root: Option<String>,
    pub tracks: Vec<(TrackRole, String, f64)>,
    pub folder_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SpeakerRow {
    pub id: String,
    pub transcript_id: String,
    pub role: SpeakerRole,
    pub number: i64,
    pub custom_name: Option<String>,
}

impl SpeakerRow {
    pub fn default_name(&self) -> String {
        default_speaker_name(self.role, self.number)
    }

    pub fn display_name(&self) -> String {
        match &self.custom_name {
            Some(name) if !name.is_empty() => name.clone(),
            _ => self.default_name(),
        }
    }
}

pub struct Library {
    conn: Connection,
}

impl Library {
    pub fn open(path: &Path) -> Result<Self> {
        Self::with_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(mut conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
        for (index, migration) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = conn.transaction()?;
            tx.execute_batch(migration)?;
            tx.execute_batch(&format!("PRAGMA user_version = {}", index + 1))?;
            tx.commit()?;
            tracing::info!(version = index + 1, "library migrated");
        }
        Ok(Self { conn })
    }

    // MARK: Recordings

    pub fn add_recording(&mut self, recording: &NewRecording) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO recordings (id, title, created_at, duration, source, status, storage_root, folder_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                recording.id,
                recording.title,
                recording.created_at.to_rfc3339(),
                recording.duration,
                recording.source.as_str(),
                TranscriptionStatus::NotTranscribed.as_str(),
                recording.storage_root,
                recording.folder_id,
            ],
        )?;
        for (role, file_name, duration) in &recording.tracks {
            tx.execute(
                "INSERT INTO tracks (id, recording_id, role, file_name, duration) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![Uuid::new_v4().to_string(), recording.id, role.as_str(), file_name, duration],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn contains(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM recordings WHERE id = ?1", params![id], |r| r.get::<_, i64>(0))? > 0)
    }

    pub fn summaries(&self) -> Result<Vec<RecordingSummary>> {
        let mut statement = self.conn.prepare(
            "SELECT r.id, r.title, r.created_at, r.duration, r.source, r.status, r.error_message,
                    COALESCE(t.language, r.language), r.folder_id, t.id IS NOT NULL
             FROM recordings r LEFT JOIN transcripts t ON t.recording_id = r.id
             ORDER BY r.created_at DESC",
        )?;
        let rows = statement.query_map([], |r| {
            Ok(RecordingSummary {
                id: r.get(0)?,
                title: r.get(1)?,
                created_at: r.get(2)?,
                duration: r.get(3)?,
                source: RecordingSource::parse(&r.get::<_, String>(4)?),
                status: TranscriptionStatus::parse(&r.get::<_, String>(5)?),
                error_message: r.get(6)?,
                language: r.get(7)?,
                folder_id: r.get(8)?,
                has_transcript: r.get(9)?,
                has_audio: false,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn summary(&self, id: &str) -> Result<Option<RecordingSummary>> {
        Ok(self.summaries()?.into_iter().find(|s| s.id == id))
    }

    pub fn recording(&self, id: &str) -> Result<Option<RecordingRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, title, created_at, duration, source, status, storage_root FROM recordings WHERE id = ?1",
                params![id],
                |r| {
                    Ok(RecordingRow {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        created_at: r
                            .get::<_, String>(2)?
                            .parse::<DateTime<Utc>>()
                            .unwrap_or_else(|_| Utc::now()),
                        duration: r.get(3)?,
                        source: RecordingSource::parse(&r.get::<_, String>(4)?),
                        status: TranscriptionStatus::parse(&r.get::<_, String>(5)?),
                        storage_root: r.get(6)?,
                    })
                },
            )
            .optional()?)
    }

    /// All recordings with their storage root, for recovery and storage checks.
    pub fn storage_roots(&self) -> Result<Vec<Option<String>>> {
        let mut statement = self.conn.prepare("SELECT DISTINCT storage_root FROM recordings")?;
        let rows = statement.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// (recording ID, storage root, file name) of every playback track.
    pub fn playback_files(&self) -> Result<Vec<(String, Option<String>, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT r.id, r.storage_root, t.file_name FROM tracks t JOIN recordings r ON r.id = t.recording_id
             WHERE t.role = 'primary'",
        )?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn tracks(&self, recording_id: &str) -> Result<Vec<TrackRow>> {
        let mut statement =
            self.conn.prepare("SELECT id, role, file_name, duration FROM tracks WHERE recording_id = ?1 ORDER BY rowid")?;
        let rows = statement.query_map(params![recording_id], |r| {
            Ok(TrackRow {
                id: r.get(0)?,
                role: TrackRole::parse(&r.get::<_, String>(1)?),
                file_name: r.get(2)?,
                duration: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn replace_track_file(&mut self, track_id: &str, file_name: &str) -> Result<()> {
        self.conn.execute("UPDATE tracks SET file_name = ?1 WHERE id = ?2", params![file_name, track_id])?;
        Ok(())
    }

    pub fn rename(&mut self, id: &str, title: &str) -> Result<()> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        self.conn.execute("UPDATE recordings SET title = ?1 WHERE id = ?2", params![trimmed, id])?;
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM recordings WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn set_status(&mut self, id: &str, status: TranscriptionStatus, error: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE recordings SET status = ?1, error_message = ?2 WHERE id = ?3",
            params![status.as_str(), error, id],
        )?;
        Ok(())
    }

    pub fn status(&self, id: &str) -> Result<Option<TranscriptionStatus>> {
        Ok(self
            .conn
            .query_row("SELECT status FROM recordings WHERE id = ?1", params![id], |r| r.get::<_, String>(0))
            .optional()?
            .map(|s| TranscriptionStatus::parse(&s)))
    }

    pub fn has_transcript(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM transcripts WHERE recording_id = ?1", params![id], |r| r.get::<_, i64>(0))? > 0)
    }

    /// Recordings waiting or running when the app quit, oldest first.
    pub fn pending_jobs(&self) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT id FROM recordings WHERE status IN ('queued', 'running') ORDER BY created_at")?;
        let rows = statement.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// IDs whose title or transcript contains the query (case- and
    /// diacritic-insensitive).
    pub fn search(&self, query: &str) -> Result<Vec<String>> {
        let key = search_key(query.trim());
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let mut ids: HashSet<String> = HashSet::new();
        let mut statement = self.conn.prepare("SELECT id, title FROM recordings")?;
        for row in statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, title) = row?;
            if search_key(&title).contains(&key) {
                ids.insert(id);
            }
        }
        let mut statement = self.conn.prepare("SELECT recording_id FROM transcripts WHERE instr(search_text, ?1) > 0")?;
        for row in statement.query_map(params![key], |r| r.get::<_, String>(0))? {
            ids.insert(row?);
        }
        Ok(ids.into_iter().collect())
    }

    // MARK: Folders

    pub fn folders(&self) -> Result<Vec<FolderInfo>> {
        let mut statement = self.conn.prepare("SELECT id, name, created_at FROM folders")?;
        let mut folders = statement
            .query_map([], |r| Ok(FolderInfo { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        folders.sort_by_key(|f| f.name.to_lowercase());
        Ok(folders)
    }

    pub fn create_folder(&mut self, name: &str) -> Result<FolderInfo> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("empty folder name"));
        }
        let folder = FolderInfo { id: Uuid::new_v4().to_string(), name: name.to_string(), created_at: now() };
        self.conn.execute(
            "INSERT INTO folders (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![folder.id, folder.name, folder.created_at],
        )?;
        Ok(folder)
    }

    pub fn rename_folder(&mut self, id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if !name.is_empty() {
            self.conn.execute("UPDATE folders SET name = ?1 WHERE id = ?2", params![name, id])?;
        }
        Ok(())
    }

    /// Deleting a folder keeps its recordings.
    pub fn delete_folder(&mut self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM folders WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Files recordings in a folder, `None` = out of any folder.
    pub fn move_to_folder(&mut self, ids: &[String], folder_id: Option<&str>) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("UPDATE recordings SET folder_id = ?1 WHERE id = ?2", params![folder_id, id])?;
        }
        tx.commit()?;
        Ok(())
    }

    // MARK: Transcripts

    pub fn transcript_id(&self, recording_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT id FROM transcripts WHERE recording_id = ?1", params![recording_id], |r| r.get(0))
            .optional()?)
    }

    /// Replaces the recording's transcript with the draft. Returns the speaker
    /// ID per draft key and the IDs of the speakers that were removed.
    pub fn store_transcript(
        &mut self,
        recording_id: &str,
        draft: &DraftTranscript,
        language: Option<&str>,
        engine: &str,
    ) -> Result<(HashMap<SpeakerKey, String>, Vec<String>)> {
        let tx = self.conn.transaction()?;
        let old_speakers: Vec<String> = {
            let mut statement = tx.prepare(
                "SELECT s.id FROM speakers s JOIN transcripts t ON t.id = s.transcript_id WHERE t.recording_id = ?1",
            )?;
            let rows = statement.query_map(params![recording_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.execute("DELETE FROM transcripts WHERE recording_id = ?1", params![recording_id])?;

        let transcript_id = Uuid::new_v4().to_string();
        let created = now();
        tx.execute(
            "INSERT INTO transcripts (id, recording_id, created_at, modified_at, language, engine)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5)",
            params![transcript_id, recording_id, created, language, engine],
        )?;

        let mut speakers: HashMap<SpeakerKey, String> = HashMap::new();
        for (index, segment) in draft.segments.iter().enumerate() {
            let speaker_id = match segment.speaker {
                Some(key) => Some(match speakers.get(&key) {
                    Some(id) => id.clone(),
                    None => {
                        let id = Uuid::new_v4().to_string();
                        tx.execute(
                            "INSERT INTO speakers (id, transcript_id, role, number) VALUES (?1, ?2, ?3, ?4)",
                            params![id, transcript_id, key.role.as_str(), key.number],
                        )?;
                        speakers.insert(key, id.clone());
                        id
                    }
                }),
                None => None,
            };
            let words = (!segment.words.is_empty()).then(|| serde_json::to_string(&segment.words)).transpose()?;
            tx.execute(
                "INSERT INTO segments (id, transcript_id, idx, start, end, text, words, speaker_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    Uuid::new_v4().to_string(),
                    transcript_id,
                    index as i64,
                    segment.start,
                    segment.end,
                    segment.text(),
                    words,
                    speaker_id
                ],
            )?;
        }
        Self::rebuild_full_text(&tx, &transcript_id)?;
        tx.execute(
            "UPDATE recordings SET language = ?1, status = 'done', error_message = NULL WHERE id = ?2",
            params![language, recording_id],
        )?;
        tx.commit()?;
        Ok((speakers, old_speakers))
    }

    /// Rebuilds the search text after segments were added or edited.
    fn rebuild_full_text(tx: &Transaction, transcript_id: &str) -> Result<()> {
        let texts: Vec<String> = {
            let mut statement = tx.prepare("SELECT text FROM segments WHERE transcript_id = ?1 ORDER BY start, idx")?;
            let rows = statement.query_map(params![transcript_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let full = texts.join("\n");
        tx.execute(
            "UPDATE transcripts SET full_text = ?1, search_text = ?2, modified_at = ?3 WHERE id = ?4",
            params![full, search_key(&full), now(), transcript_id],
        )?;
        Ok(())
    }

    fn touch(&mut self, transcript_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        Self::rebuild_full_text(&tx, transcript_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn transcript_view(&self, recording_id: &str) -> Result<Option<TranscriptView>> {
        let Some((id, language, engine, modified_at)) = self
            .conn
            .query_row(
                "SELECT id, language, engine, modified_at FROM transcripts WHERE recording_id = ?1",
                params![recording_id],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let speakers = self
            .speakers(&id)?
            .into_iter()
            .map(|s| SpeakerView {
                display_name: s.display_name(),
                default_name: s.default_name(),
                id: s.id,
                role: s.role,
                number: s.number,
                custom_name: s.custom_name,
            })
            .collect();
        let mut statement = self.conn.prepare(
            "SELECT id, idx, start, end, text, original_text IS NOT NULL, speaker_id
             FROM segments WHERE transcript_id = ?1 ORDER BY start, idx",
        )?;
        let segments = statement
            .query_map(params![id], |r| {
                Ok(SegmentView {
                    id: r.get(0)?,
                    index: r.get(1)?,
                    start: r.get(2)?,
                    end: r.get(3)?,
                    text: r.get(4)?,
                    is_edited: r.get(5)?,
                    speaker_id: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(TranscriptView { id, language, engine, modified_at, speakers, segments }))
    }

    pub fn speakers(&self, transcript_id: &str) -> Result<Vec<SpeakerRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, transcript_id, role, number, custom_name FROM speakers WHERE transcript_id = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![transcript_id], |r| {
            Ok(SpeakerRow {
                id: r.get(0)?,
                transcript_id: r.get(1)?,
                role: SpeakerRole::parse(&r.get::<_, String>(2)?),
                number: r.get(3)?,
                custom_name: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn speaker(&self, id: &str) -> Result<Option<SpeakerRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, transcript_id, role, number, custom_name FROM speakers WHERE id = ?1",
                params![id],
                |r| {
                    Ok(SpeakerRow {
                        id: r.get(0)?,
                        transcript_id: r.get(1)?,
                        role: SpeakerRole::parse(&r.get::<_, String>(2)?),
                        number: r.get(3)?,
                        custom_name: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Segment IDs of a speaker, in playback order.
    pub fn segments_of(&self, speaker_id: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare("SELECT id FROM segments WHERE speaker_id = ?1 ORDER BY start, idx")?;
        let rows = statement.query_map(params![speaker_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// (transcript ID, speaker ID) of a segment.
    pub fn segment_owner(&self, segment_id: &str) -> Result<Option<(String, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT transcript_id, speaker_id FROM segments WHERE id = ?1",
                params![segment_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn recording_of_transcript(&self, transcript_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT recording_id FROM transcripts WHERE id = ?1", params![transcript_id], |r| r.get(0))
            .optional()?)
    }

    /// Seconds spoken per speaker.
    pub fn speech_seconds(&self, transcript_id: &str) -> Result<HashMap<String, f64>> {
        let mut statement = self.conn.prepare(
            "SELECT speaker_id, SUM(end - start) FROM segments WHERE transcript_id = ?1 AND speaker_id IS NOT NULL GROUP BY speaker_id",
        )?;
        let rows = statement.query_map(params![transcript_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    }

    // MARK: Editing

    /// Saves a manual correction; the engine's text is kept for "restore".
    pub fn edit_segment(&mut self, segment_id: &str, text: &str) -> Result<()> {
        let trimmed = text.trim();
        let Some((transcript_id, current, original)) = self
            .conn
            .query_row(
                "SELECT transcript_id, text, original_text FROM segments WHERE id = ?1",
                params![segment_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)),
            )
            .optional()?
        else {
            return Ok(());
        };
        if trimmed.is_empty() || trimmed == current {
            return Ok(());
        }
        self.conn.execute(
            "UPDATE segments SET text = ?1, original_text = ?2 WHERE id = ?3",
            params![trimmed, original.unwrap_or(current), segment_id],
        )?;
        self.touch(&transcript_id)
    }

    pub fn restore_segment(&mut self, segment_id: &str) -> Result<()> {
        let Some((transcript_id, original)) = self
            .conn
            .query_row(
                "SELECT transcript_id, original_text FROM segments WHERE id = ?1",
                params![segment_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?
        else {
            return Ok(());
        };
        let Some(original) = original else { return Ok(()) };
        self.conn.execute(
            "UPDATE segments SET text = ?1, original_text = NULL WHERE id = ?2",
            params![original, segment_id],
        )?;
        self.touch(&transcript_id)
    }

    pub fn segment_text(&self, segment_id: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT text FROM segments WHERE id = ?1", params![segment_id], |r| r.get(0)).optional()?)
    }

    pub fn set_custom_name(&mut self, speaker_id: &str, name: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE speakers SET custom_name = ?1 WHERE id = ?2", params![name, speaker_id])?;
        if let Some(speaker) = self.speaker(speaker_id)? {
            self.touch(&speaker.transcript_id)?;
        }
        Ok(())
    }

    /// Moves all segments of `speaker_id` to `target_id` and removes the speaker.
    pub fn merge_speaker(&mut self, speaker_id: &str, target_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE segments SET speaker_id = ?1 WHERE speaker_id = ?2", params![target_id, speaker_id])?;
        tx.execute("DELETE FROM speakers WHERE id = ?1", params![speaker_id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_segment_speaker(&mut self, segment_id: &str, speaker_id: &str) -> Result<()> {
        self.conn.execute("UPDATE segments SET speaker_id = ?1 WHERE id = ?2", params![speaker_id, segment_id])?;
        Ok(())
    }

    pub fn delete_speaker(&mut self, speaker_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM speakers WHERE id = ?1", params![speaker_id])?;
        Ok(())
    }

    pub fn add_speaker(&mut self, transcript_id: &str, role: SpeakerRole, number: i64, name: Option<&str>) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO speakers (id, transcript_id, role, number, custom_name) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, transcript_id, role.as_str(), number, name],
        )?;
        Ok(id)
    }

    pub fn touch_transcript(&mut self, transcript_id: &str) -> Result<()> {
        self.touch(transcript_id)
    }

    #[cfg(test)]
    pub(crate) fn conn_for_tests(&self) -> &Connection {
        &self.conn
    }

    // MARK: Export

    /// The transcript as plain values; speaker names only if it has more
    /// than one speaker in use.
    pub fn export_document(&self, recording_id: &str) -> Result<Option<ExportDocument>> {
        let Some(recording) = self.recording(recording_id)? else { return Ok(None) };
        let Some((transcript_id, language, engine)) = self
            .conn
            .query_row(
                "SELECT id, language, engine FROM transcripts WHERE recording_id = ?1",
                params![recording_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let names: HashMap<String, String> =
            self.speakers(&transcript_id)?.into_iter().map(|s| (s.id.clone(), s.display_name())).collect();
        let mut statement = self.conn.prepare(
            "SELECT start, end, text, speaker_id, words, original_text IS NOT NULL
             FROM segments WHERE transcript_id = ?1 ORDER BY start, idx",
        )?;
        let rows: Vec<(f64, f64, String, Option<String>, Option<String>, bool)> = statement
            .query_map(params![transcript_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            return Ok(None);
        }
        let used: HashSet<&String> = rows.iter().filter_map(|r| r.3.as_ref()).collect();
        let shows_speakers = used.len() > 1;
        let segments = rows
            .iter()
            .map(|(start, end, text, speaker, words, edited)| ExportSegment {
                start: *start,
                end: *end,
                text: text.clone(),
                speaker: if shows_speakers { speaker.as_ref().and_then(|s| names.get(s).cloned()) } else { None },
                // After a manual correction the word timings no longer match the text.
                words: if *edited {
                    Vec::new()
                } else {
                    words.as_deref().and_then(|w| serde_json::from_str::<Vec<WordTiming>>(w).ok()).unwrap_or_default()
                },
            })
            .collect();
        Ok(Some(ExportDocument {
            title: recording.title,
            created_at: recording.created_at,
            duration: recording.duration,
            language,
            engine,
            segments,
        }))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::transcript::SegmentDraft;

    pub(crate) fn new_recording(library: &mut Library, title: &str, source: RecordingSource) -> String {
        let id = Uuid::new_v4().to_string();
        library
            .add_recording(&NewRecording {
                id: id.clone(),
                title: title.into(),
                created_at: Utc::now(),
                duration: 60.0,
                source,
                storage_root: None,
                tracks: vec![(TrackRole::Primary, "recording.m4a".into(), 60.0)],
                folder_id: None,
            })
            .unwrap();
        id
    }

    pub(crate) fn draft(texts: &[(&str, Option<SpeakerKey>)]) -> DraftTranscript {
        DraftTranscript {
            segments: texts
                .iter()
                .enumerate()
                .map(|(i, (text, speaker))| SegmentDraft {
                    start: i as f64 * 10.0,
                    end: i as f64 * 10.0 + 9.0,
                    words: text
                        .split(' ')
                        .enumerate()
                        .map(|(j, w)| WordTiming::new(w, i as f64 * 10.0 + j as f64 * 0.5, i as f64 * 10.0 + j as f64 * 0.5 + 0.4))
                        .collect(),
                    speaker: *speaker,
                })
                .collect(),
            voices: HashMap::new(),
        }
    }

    #[test]
    fn migrations_run_once() {
        let library = Library::open_in_memory().unwrap();
        let version: i64 = library.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
    }

    #[test]
    fn recordings_folders_and_search() {
        let mut library = Library::open_in_memory().unwrap();
        let first = new_recording(&mut library, "Wöchentliches Meeting", RecordingSource::Meeting);
        let second = new_recording(&mut library, "Notiz", RecordingSource::Microphone);
        library.store_transcript(&second, &draft(&[("Das Budget für Köln.", None)]), Some("de"), "test").unwrap();

        assert_eq!(library.search("wochentliches").unwrap(), [first.clone()]);
        assert_eq!(library.search("KOLN").unwrap(), [second.clone()]);
        assert!(library.search("nichts").unwrap().is_empty());

        let folder = library.create_folder("Projekte").unwrap();
        library.move_to_folder(&[first.clone()], Some(&folder.id)).unwrap();
        assert_eq!(library.summary(&first).unwrap().unwrap().folder_id.as_deref(), Some(folder.id.as_str()));
        library.delete_folder(&folder.id).unwrap();
        let summary = library.summary(&first).unwrap().unwrap();
        assert!(summary.folder_id.is_none());

        let done = library.summary(&second).unwrap().unwrap();
        assert_eq!(done.status, TranscriptionStatus::Done);
        assert!(done.has_transcript);
        library.delete(&second).unwrap();
        assert!(library.transcript_id(&second).unwrap().is_none());
    }

    #[test]
    fn edits_keep_the_original_text() {
        let mut library = Library::open_in_memory().unwrap();
        let id = new_recording(&mut library, "Notiz", RecordingSource::Microphone);
        library.store_transcript(&id, &draft(&[("Hallo Welt.", None)]), None, "test").unwrap();
        let segment = library.transcript_view(&id).unwrap().unwrap().segments[0].id.clone();
        library.edit_segment(&segment, "Hallo Welt!").unwrap();
        library.edit_segment(&segment, "Hallo, Welt!").unwrap();
        let view = library.transcript_view(&id).unwrap().unwrap();
        assert!(view.segments[0].is_edited);
        assert_eq!(library.search("hallo, welt").unwrap(), [id.clone()]);
        // Edited segments lose their word timings in exports.
        assert!(library.export_document(&id).unwrap().unwrap().segments[0].words.is_empty());
        library.restore_segment(&segment).unwrap();
        let view = library.transcript_view(&id).unwrap().unwrap();
        assert_eq!(view.segments[0].text, "Hallo Welt.");
        assert!(!view.segments[0].is_edited);
    }

    #[test]
    fn export_names_speakers_only_with_several() {
        let mut library = Library::open_in_memory().unwrap();
        let id = new_recording(&mut library, "Meeting", RecordingSource::Meeting);
        library
            .store_transcript(&id, &draft(&[("Hallo.", Some(SpeakerKey::ME)), ("Hi.", Some(SpeakerKey::diarized(1)))]), None, "test")
            .unwrap();
        let document = library.export_document(&id).unwrap().unwrap();
        assert_eq!(document.segments[0].speaker.as_deref(), Some("Ich"));
        assert_eq!(document.segments[1].speaker.as_deref(), Some("Sprecher 1"));
        assert_eq!(document.segments[0].words.len(), 1);

        let single = new_recording(&mut library, "Notiz", RecordingSource::Microphone);
        library.store_transcript(&single, &draft(&[("Hallo.", Some(SpeakerKey::ME))]), None, "test").unwrap();
        assert!(library.export_document(&single).unwrap().unwrap().segments[0].speaker.is_none());
    }
}
