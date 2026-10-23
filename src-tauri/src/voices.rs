//! Remembers people's voices and recognizes them in new transcripts.
//! A port of Scripta's `VoiceMatcher` and `VoiceLibrary`; keep them in step.
//!
//! Voice fingerprints are biometric data, so they live in a database file of
//! their own (`voices.sqlite`), apart from the library: deleting all voice
//! profiles empties this file, nothing else. The anonymous voice of each
//! transcript speaker is always kept (it is derived from the recording and
//! deleted with it), so speakers can still be learned from after the user
//! turns recognition on. People, their samples and any matching exist only
//! once the user turned voice recognition on.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

// MARK: - Matching

/// Below this, a speaker stays unknown.
pub const SUGGESTION_THRESHOLD: f32 = 0.5;
/// From this on (and with the conditions below), the name is set automatically.
pub const AUTOMATIC_THRESHOLD: f32 = 0.7;
/// How much better than the second-best person an automatic match must be.
pub const AUTOMATIC_MARGIN: f32 = 0.1;
/// Confirmed samples a person needs before names are set automatically.
pub const CONFIRMED_SAMPLES_FOR_AUTOMATIC: usize = 3;
/// Shorter speakers are too uncertain to match at all.
pub const MINIMUM_SPEECH_FOR_MATCH: f64 = 3.0;
pub const MINIMUM_SPEECH_FOR_AUTOMATIC: f64 = 10.0;
/// Shorter speakers don't become samples (their embedding is noisy).
pub const MINIMUM_SPEECH_FOR_SAMPLE: f64 = 5.0;
/// Samples compared per person.
pub const BEST_SAMPLES: usize = 3;
/// Oldest samples beyond this are dropped (unconfirmed ones first).
pub const MAXIMUM_SAMPLES: usize = 40;
/// A new sample this similar to an existing one replaces it: the same
/// recording transcribed again.
pub const DUPLICATE_SIMILARITY: f32 = 0.995;

pub struct MatchVoice {
    pub id: String,
    pub embedding: Vec<f32>,
    pub speech_seconds: f64,
}

pub struct Candidate {
    pub id: String,
    pub samples: Vec<Vec<f32>>,
    pub confirmed_samples: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub person: String,
    pub score: f32,
    pub is_automatic: bool,
}

/// Decides which known person a transcript speaker is.
///
/// A speaker is compared with each person's samples (cosine similarity, mean
/// of the best three: people sound different over Teams, in a room or on a
/// headset, and each sample covers one of those). Every person gets at most
/// one speaker per transcript. A name is only given automatically when the
/// match is strong, clearly better than the next person, and the person has
/// been confirmed by the user a few times; otherwise it stays a suggestion.
pub fn match_voices(voices: &[MatchVoice], people: &[Candidate]) -> HashMap<String, Match> {
    let eligible: Vec<&MatchVoice> = voices
        .iter()
        .filter(|v| v.speech_seconds >= MINIMUM_SPEECH_FOR_MATCH && !v.embedding.is_empty())
        .collect();
    let mut scores: HashMap<&str, HashMap<&str, f32>> = HashMap::new();
    for voice in &eligible {
        for person in people.iter().filter(|p| !p.samples.is_empty()) {
            scores.entry(&voice.id).or_default().insert(&person.id, score(&voice.embedding, &person.samples));
        }
    }

    // Strongest pairs first; each voice and each person is used once.
    let mut pairs: Vec<(&str, &str, f32)> = scores
        .iter()
        .flat_map(|(voice, by_person)| {
            by_person
                .iter()
                .filter(|(_, s)| **s >= SUGGESTION_THRESHOLD)
                .map(move |(person, s)| (*voice, *person, *s))
        })
        .collect();
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut matches: HashMap<String, Match> = HashMap::new();
    let mut taken: HashSet<&str> = HashSet::new();
    for (voice_id, person_id, pair_score) in pairs {
        if matches.contains_key(voice_id) || taken.contains(person_id) {
            continue;
        }
        let (Some(voice), Some(person)) =
            (eligible.iter().find(|v| v.id == voice_id), people.iter().find(|p| p.id == person_id))
        else {
            continue;
        };
        let runner_up = scores[voice_id]
            .iter()
            .filter(|(p, _)| **p != person_id)
            .map(|(_, s)| *s)
            .fold(-1.0_f32, f32::max);
        let is_automatic = pair_score >= AUTOMATIC_THRESHOLD
            && pair_score - runner_up >= AUTOMATIC_MARGIN
            && person.confirmed_samples >= CONFIRMED_SAMPLES_FOR_AUTOMATIC
            && voice.speech_seconds >= MINIMUM_SPEECH_FOR_AUTOMATIC;
        matches.insert(voice_id.to_string(), Match { person: person_id.to_string(), score: pair_score, is_automatic });
        taken.insert(person_id);
    }
    matches
}

/// Mean similarity to the person's closest samples.
pub fn score(embedding: &[f32], samples: &[Vec<f32>]) -> f32 {
    let mut similarities: Vec<f32> = samples.iter().map(|s| cosine(embedding, s)).collect();
    if similarities.is_empty() {
        return -1.0;
    }
    similarities.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let best = &similarities[..similarities.len().min(BEST_SAMPLES)];
    best.iter().sum::<f32>() / best.len() as f32
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return -1.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norms = (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if norms > 0.0 {
        dot / norms
    } else {
        -1.0
    }
}

// MARK: - Store

/// How a transcript speaker relates to a known person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchState {
    /// Not recognized.
    Unknown,
    /// Probably this person: the name is set but marked as a guess ("?")
    /// until the user confirms it. Guesses teach nothing.
    Suggested,
    /// Recognized with confidence and named automatically.
    Automatic,
    /// Named or confirmed by the user.
    Confirmed,
}

impl MatchState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Suggested => "suggested",
            Self::Automatic => "automatic",
            Self::Confirmed => "confirmed",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "suggested" => Self::Suggested,
            "automatic" => Self::Automatic,
            "confirmed" => Self::Confirmed,
            _ => Self::Unknown,
        }
    }
}

/// A known person, for lists and menus.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonSummary {
    pub id: String,
    pub name: String,
    pub samples: usize,
    pub confirmed_samples: usize,
    pub last_heard: Option<String>,
}

/// What Scripta knows about the voice of a transcript speaker.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub person_id: Option<String>,
    pub person_name: Option<String>,
    pub state: MatchState,
    pub score: Option<f64>,
    pub speech_seconds: f64,
}

/// The voice of a speaker in a new transcript.
pub struct LabelVoice {
    pub label_id: String,
    pub embedding: Vec<f32>,
    pub speech_seconds: f64,
}

struct SpeakerVoiceRow {
    label_id: String,
    recording_id: String,
    embedding: Vec<f32>,
    speech_seconds: f64,
    person_id: Option<String>,
    state: MatchState,
    score: Option<f64>,
    sample_id: Option<String>,
}

struct SampleRow {
    id: String,
    embedding: Vec<f32>,
    label_id: Option<String>,
    is_confirmed: bool,
}

pub struct VoiceStore {
    conn: Connection,
}

fn to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn from_blob(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl VoiceStore {
    pub fn open(path: &Path) -> Result<Self> {
        Self::with_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "BEGIN;
                CREATE TABLE people (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE samples (
                    id TEXT PRIMARY KEY,
                    person_id TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
                    embedding BLOB NOT NULL,
                    speech_seconds REAL NOT NULL,
                    is_confirmed INTEGER NOT NULL,
                    recording_id TEXT,
                    label_id TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX samples_person ON samples(person_id);
                CREATE TABLE speaker_voices (
                    label_id TEXT PRIMARY KEY,
                    recording_id TEXT NOT NULL,
                    embedding BLOB NOT NULL,
                    speech_seconds REAL NOT NULL,
                    person_id TEXT,
                    state TEXT NOT NULL,
                    score REAL,
                    sample_id TEXT
                );
                CREATE INDEX speaker_voices_recording ON speaker_voices(recording_id);
                PRAGMA user_version = 1;
                COMMIT;",
            )?;
        }
        Ok(Self { conn })
    }

    // MARK: Recognition

    /// Stores the voices of a new transcript and matches them with known
    /// people. Returns the names to set by label ID: confident matches and
    /// guesses alike (guesses are marked in the UI and teach nothing until
    /// confirmed).
    pub fn recognize(&mut self, voices: &[LabelVoice], recording_id: &str, enabled: bool) -> Result<HashMap<String, String>> {
        if voices.is_empty() {
            return Ok(HashMap::new());
        }
        let everyone = if enabled { self.all_people()? } else { Vec::new() };
        let mut candidates = Vec::new();
        for (id, _) in &everyone {
            let samples = self.samples_of(id)?;
            candidates.push(Candidate {
                id: id.clone(),
                confirmed_samples: samples.iter().filter(|s| s.is_confirmed).count(),
                samples: samples.into_iter().map(|s| s.embedding).collect(),
            });
        }
        let matches = match_voices(
            &voices
                .iter()
                .map(|v| MatchVoice { id: v.label_id.clone(), embedding: v.embedding.clone(), speech_seconds: v.speech_seconds })
                .collect::<Vec<_>>(),
            &candidates,
        );

        let mut names = HashMap::new();
        for voice in voices {
            let mut row = SpeakerVoiceRow {
                label_id: voice.label_id.clone(),
                recording_id: recording_id.to_string(),
                embedding: voice.embedding.clone(),
                speech_seconds: voice.speech_seconds,
                person_id: None,
                state: MatchState::Unknown,
                score: None,
                sample_id: None,
            };
            if let Some(found) = matches.get(&voice.label_id) {
                if let Some((person_id, name)) = everyone.iter().find(|(id, _)| *id == found.person) {
                    row.person_id = Some(person_id.clone());
                    row.score = Some(found.score as f64);
                    if found.is_automatic {
                        row.state = MatchState::Automatic;
                        row.sample_id = self.add_sample(person_id, &row, false)?;
                    } else {
                        row.state = MatchState::Suggested;
                    }
                    names.insert(voice.label_id.clone(), name.clone());
                    tracing::debug!(score = found.score, automatic = found.is_automatic, "speaker matched");
                }
            }
            self.save_voice(&row)?;
        }
        Ok(names)
    }

    // MARK: Naming

    /// The user named a speaker: links it to the person with that name
    /// (created if new) and learns the voice.
    pub fn link(&mut self, label_id: &str, name: &str, enabled: bool) -> Result<()> {
        if !enabled {
            return Ok(());
        }
        let Some(mut row) = self.voice(label_id)? else { return Ok(()) };
        let person_id = match self.person_named(name)? {
            Some(id) => id,
            None => {
                let id = Uuid::new_v4().to_string();
                self.conn.execute("INSERT INTO people (id, name, created_at) VALUES (?1, ?2, ?3)", params![id, name, now()])?;
                id
            }
        };
        if row.person_id.as_deref() != Some(person_id.as_str()) {
            self.remove_sample(&mut row)?;
        }
        self.remove_copies(&row, &person_id)?;
        row.person_id = Some(person_id.clone());
        row.state = MatchState::Confirmed;
        row.sample_id = self.add_sample(&person_id, &row, true)?;
        self.save_voice(&row)
    }

    /// One voice belongs to one person: samples of the same voice in other
    /// profiles are removed, or the two people would be confused from then on.
    fn remove_copies(&mut self, voice: &SpeakerVoiceRow, person_id: &str) -> Result<()> {
        for (other, _) in self.all_people()? {
            if other == person_id {
                continue;
            }
            for sample in self.samples_of(&other)? {
                if sample.label_id.as_deref() == Some(voice.label_id.as_str())
                    || cosine(&sample.embedding, &voice.embedding) >= DUPLICATE_SIMILARITY
                {
                    self.conn.execute("DELETE FROM samples WHERE id = ?1", params![sample.id])?;
                }
            }
        }
        Ok(())
    }

    /// The speaker is nobody to remember (a guest) or was renamed back: drops
    /// the link and the sample it gave.
    pub fn unlink(&mut self, label_id: &str) -> Result<()> {
        let Some(mut row) = self.voice(label_id)? else { return Ok(()) };
        if row.person_id.is_none() {
            return Ok(());
        }
        self.remove_sample(&mut row)?;
        row.person_id = None;
        row.score = None;
        row.state = MatchState::Unknown;
        self.save_voice(&row)
    }

    /// Two speakers of a transcript are one person: the merged speaker's voice
    /// becomes another sample of the target's person, but only if it sounds
    /// like that person. Merging is also how users clean up a mix-up, and a
    /// dissimilar voice would spoil the profile.
    pub fn merge(&mut self, label_id: &str, target_id: &str, enabled: bool) -> Result<()> {
        let Some(mut source) = self.voice(label_id)? else { return Ok(()) };
        let mut taught = false;
        if enabled {
            if let Some(target) = self.voice(target_id)? {
                if matches!(target.state, MatchState::Confirmed | MatchState::Automatic) {
                    if let Some(person_id) = target.person_id.clone() {
                        let samples: Vec<Vec<f32>> = self.samples_of(&person_id)?.into_iter().map(|s| s.embedding).collect();
                        if !samples.is_empty() && score(&source.embedding, &samples) >= SUGGESTION_THRESHOLD {
                            if source.person_id.as_deref() != Some(person_id.as_str()) {
                                self.remove_sample(&mut source)?;
                            }
                            self.add_sample(&person_id, &source, true)?;
                            taught = true;
                        }
                    }
                }
            }
        }
        if !taught {
            self.remove_sample(&mut source)?;
        }
        self.conn.execute("DELETE FROM speaker_voices WHERE label_id = ?1", params![label_id])?;
        Ok(())
    }

    /// The speaker turned out to mix several people (the user moved passages
    /// out of it): its voice is no fingerprint of one person, so it is
    /// dropped, together with any sample it gave.
    pub fn discard_voice(&mut self, label_id: &str) -> Result<()> {
        let Some(mut row) = self.voice(label_id)? else { return Ok(()) };
        self.remove_sample(&mut row)?;
        self.conn.execute("DELETE FROM speaker_voices WHERE label_id = ?1", params![label_id])?;
        Ok(())
    }

    /// The speakers no longer exist (transcript replaced or deleted). Their
    /// samples stay: they belong to the people now.
    pub fn forget(&mut self, label_ids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in label_ids {
            tx.execute("DELETE FROM speaker_voices WHERE label_id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn forget_recording(&mut self, recording_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM speaker_voices WHERE recording_id = ?1", params![recording_id])?;
        Ok(())
    }

    // MARK: People

    /// Renames a person. Returns the speakers that carry the name, so the
    /// caller can update them.
    pub fn rename(&mut self, person_id: &str, name: &str) -> Result<Vec<String>> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let changed = self.conn.execute("UPDATE people SET name = ?1 WHERE id = ?2", params![trimmed, person_id])?;
        if changed == 0 {
            return Ok(Vec::new());
        }
        self.named_labels(person_id)
    }

    /// Speakers named after the person (automatically or by the user).
    pub fn named_labels(&self, person_id: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT label_id FROM speaker_voices WHERE person_id = ?1 AND state IN ('automatic', 'confirmed')",
        )?;
        let rows = statement.query_map(params![person_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    }

    /// Deletes a person and their samples. Transcripts keep the names.
    pub fn delete_person(&mut self, person_id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE speaker_voices SET person_id = NULL, score = NULL, sample_id = NULL, state = 'unknown' WHERE person_id = ?1",
            params![person_id],
        )?;
        tx.execute("DELETE FROM people WHERE id = ?1", params![person_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes every voice profile and fingerprint. Transcripts keep the names.
    pub fn delete_all(&mut self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM samples; DELETE FROM people; DELETE FROM speaker_voices; VACUUM;")?;
        Ok(())
    }

    /// Known people, sorted by name.
    pub fn people(&self) -> Result<Vec<PersonSummary>> {
        let mut statement = self.conn.prepare(
            "SELECT p.id, p.name,
                    (SELECT COUNT(*) FROM samples s WHERE s.person_id = p.id),
                    (SELECT COUNT(*) FROM samples s WHERE s.person_id = p.id AND s.is_confirmed = 1),
                    (SELECT MAX(created_at) FROM samples s WHERE s.person_id = p.id)
             FROM people p",
        )?;
        let mut people = statement
            .query_map([], |r| {
                Ok(PersonSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    samples: r.get::<_, i64>(2)? as usize,
                    confirmed_samples: r.get::<_, i64>(3)? as usize,
                    last_heard: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        people.sort_by_key(|p| p.name.to_lowercase());
        Ok(people)
    }

    /// Voice status of the given speakers (missing ones have no voice).
    pub fn statuses(&self, label_ids: &[String]) -> Result<HashMap<String, VoiceStatus>> {
        let names: HashMap<String, String> = self.all_people()?.into_iter().collect();
        let mut statuses = HashMap::new();
        for id in label_ids {
            if let Some(row) = self.voice(id)? {
                statuses.insert(
                    id.clone(),
                    VoiceStatus {
                        person_name: row.person_id.as_ref().and_then(|p| names.get(p).cloned()),
                        person_id: row.person_id,
                        state: row.state,
                        score: row.score,
                        speech_seconds: row.speech_seconds,
                    },
                );
            }
        }
        Ok(statuses)
    }

    /// True if anything at all is stored (for "Alle Stimmprofile löschen").
    pub fn has_data(&self) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM people) + (SELECT COUNT(*) FROM speaker_voices)",
            [],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    // MARK: Samples

    /// Adds a sample of the speaker's voice to the person (or refreshes the
    /// one it already gave). Short speakers give no sample.
    fn add_sample(&mut self, person_id: &str, voice: &SpeakerVoiceRow, is_confirmed: bool) -> Result<Option<String>> {
        if voice.speech_seconds < MINIMUM_SPEECH_FOR_SAMPLE {
            return Ok(None);
        }
        let existing = self.samples_of(person_id)?.into_iter().find(|s| {
            s.label_id.as_deref() == Some(voice.label_id.as_str()) || cosine(&s.embedding, &voice.embedding) >= DUPLICATE_SIMILARITY
        });
        if let Some(existing) = existing {
            self.conn.execute(
                "UPDATE samples SET embedding = ?1, speech_seconds = ?2, is_confirmed = ?3, label_id = ?4 WHERE id = ?5",
                params![
                    to_blob(&voice.embedding),
                    voice.speech_seconds,
                    existing.is_confirmed || is_confirmed,
                    voice.label_id,
                    existing.id
                ],
            )?;
            return Ok(Some(existing.id));
        }
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO samples (id, person_id, embedding, speech_seconds, is_confirmed, recording_id, label_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                person_id,
                to_blob(&voice.embedding),
                voice.speech_seconds,
                is_confirmed,
                voice.recording_id,
                voice.label_id,
                now()
            ],
        )?;
        self.trim_samples(person_id)?;
        Ok(Some(id))
    }

    fn trim_samples(&mut self, person_id: &str) -> Result<()> {
        let count: i64 =
            self.conn.query_row("SELECT COUNT(*) FROM samples WHERE person_id = ?1", params![person_id], |r| r.get(0))?;
        let excess = count - MAXIMUM_SAMPLES as i64;
        if excess > 0 {
            // Unconfirmed first, then the oldest (rowid follows insertion).
            self.conn.execute(
                "DELETE FROM samples WHERE id IN (
                    SELECT id FROM samples WHERE person_id = ?1 ORDER BY is_confirmed, rowid LIMIT ?2)",
                params![person_id, excess],
            )?;
        }
        Ok(())
    }

    fn remove_sample(&mut self, voice: &mut SpeakerVoiceRow) -> Result<()> {
        if let Some(sample_id) = voice.sample_id.take() {
            self.conn.execute("DELETE FROM samples WHERE id = ?1", params![sample_id])?;
        }
        Ok(())
    }

    // MARK: Rows

    fn voice(&self, label_id: &str) -> Result<Option<SpeakerVoiceRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT label_id, recording_id, embedding, speech_seconds, person_id, state, score, sample_id
                 FROM speaker_voices WHERE label_id = ?1",
                params![label_id],
                |r| {
                    Ok(SpeakerVoiceRow {
                        label_id: r.get(0)?,
                        recording_id: r.get(1)?,
                        embedding: from_blob(&r.get::<_, Vec<u8>>(2)?),
                        speech_seconds: r.get(3)?,
                        person_id: r.get(4)?,
                        state: MatchState::parse(&r.get::<_, String>(5)?),
                        score: r.get(6)?,
                        sample_id: r.get(7)?,
                    })
                },
            )
            .optional()?)
    }

    fn save_voice(&mut self, row: &SpeakerVoiceRow) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO speaker_voices
                (label_id, recording_id, embedding, speech_seconds, person_id, state, score, sample_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.label_id,
                row.recording_id,
                to_blob(&row.embedding),
                row.speech_seconds,
                row.person_id,
                row.state.as_str(),
                row.score,
                row.sample_id
            ],
        )?;
        Ok(())
    }

    fn samples_of(&self, person_id: &str) -> Result<Vec<SampleRow>> {
        let mut statement =
            self.conn.prepare("SELECT id, embedding, label_id, is_confirmed FROM samples WHERE person_id = ?1 ORDER BY rowid")?;
        let rows = statement.query_map(params![person_id], |r| {
            Ok(SampleRow {
                id: r.get(0)?,
                embedding: from_blob(&r.get::<_, Vec<u8>>(1)?),
                label_id: r.get(2)?,
                is_confirmed: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// (id, name) of everyone.
    fn all_people(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare("SELECT id, name FROM people ORDER BY rowid")?;
        let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn person_named(&self, name: &str) -> Result<Option<String>> {
        let wanted = name.trim().to_lowercase();
        Ok(self.all_people()?.into_iter().find(|(_, n)| n.to_lowercase() == wanted).map(|(id, _)| id))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Synthetic 256-d voices: each "person" is a random direction;
    /// recordings of that person add a little noise (cosine ≈ 0.96),
    /// different people are nearly orthogonal.
    pub(crate) struct SyntheticVoices {
        state: u64,
    }

    impl SyntheticVoices {
        pub(crate) fn new() -> Self {
            Self { state: 42 }
        }

        /// SplitMix64.
        fn next(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn uniform(&mut self, low: f32, high: f32) -> f32 {
            let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
            low + (high - low) * unit
        }

        pub(crate) fn person(&mut self) -> Vec<f32> {
            let v: Vec<f32> = (0..256).map(|_| self.uniform(-1.0, 1.0)).collect();
            normalized(v)
        }

        pub(crate) fn recording(&mut self, person: &[f32], noise: f32) -> Vec<f32> {
            let v: Vec<f32> = person.iter().map(|x| x + self.uniform(-noise, noise) * 0.1).collect();
            normalized(v)
        }
    }

    fn normalized(v: Vec<f32>) -> Vec<f32> {
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / norm).collect()
    }

    fn voice_of(embedding: Vec<f32>, seconds: f64) -> LabelVoice {
        LabelVoice { label_id: Uuid::new_v4().to_string(), embedding, speech_seconds: seconds }
    }

    fn candidate(samples: Vec<Vec<f32>>, confirmed: usize) -> Candidate {
        Candidate { id: Uuid::new_v4().to_string(), samples, confirmed_samples: confirmed }
    }

    fn match_voice(embedding: Vec<f32>, seconds: f64) -> MatchVoice {
        MatchVoice { id: Uuid::new_v4().to_string(), embedding, speech_seconds: seconds }
    }

    #[test]
    fn one_sample_gives_a_suggestion_not_a_name() {
        let mut synthetic = SyntheticVoices::new();
        let max = synthetic.person();
        let voice = match_voice(synthetic.recording(&max, 0.3), 60.0);
        assert!(match_voices(std::slice::from_ref(&voice), &[]).is_empty());
        let person = candidate(vec![synthetic.recording(&max, 0.3)], 1);
        let found = &match_voices(std::slice::from_ref(&voice), std::slice::from_ref(&person))[&voice.id];
        assert_eq!(found.person, person.id);
        assert!(!found.is_automatic);
    }

    #[test]
    fn after_three_confirmations_a_clear_match_is_automatic() {
        let mut synthetic = SyntheticVoices::new();
        let max = synthetic.person();
        let anna = synthetic.person();
        let people = [
            candidate((0..3).map(|_| synthetic.recording(&max, 0.3)).collect(), 3),
            candidate(vec![synthetic.recording(&anna, 0.3)], 1),
        ];
        let long = match_voice(synthetic.recording(&max, 0.3), 60.0);
        let short = match_voice(synthetic.recording(&anna, 0.3), 6.0);
        let matches = match_voices(&[long, short], &people);
        let long = matches.values().find(|m| m.person == people[0].id).unwrap();
        assert!(long.is_automatic);
        let short = matches.values().find(|m| m.person == people[1].id).unwrap();
        assert!(!short.is_automatic);
    }

    #[test]
    fn each_person_goes_to_one_speaker_only() {
        let mut synthetic = SyntheticVoices::new();
        let max = synthetic.person();
        let person = candidate(vec![max.clone()], 5);
        let close = match_voice(synthetic.recording(&max, 0.1), 60.0);
        let farther = match_voice(synthetic.recording(&max, 1.5), 60.0);
        let (close_id, farther_id) = (close.id.clone(), farther.id.clone());
        let matches = match_voices(&[farther, close], &[person]);
        assert!(matches.contains_key(&close_id));
        assert!(!matches.contains_key(&farther_id));
    }

    #[test]
    fn short_speakers_and_similar_people_are_never_automatic() {
        let mut synthetic = SyntheticVoices::new();
        let max = synthetic.person();
        let twin = synthetic.recording(&max, 0.3);
        let people = [candidate(vec![max.clone(), max.clone(), max.clone()], 3), candidate(vec![twin], 3)];
        let voice = match_voice(synthetic.recording(&max, 0.3), 60.0);
        let id = voice.id.clone();
        assert!(!match_voices(&[voice], &people)[&id].is_automatic);
        let tiny = match_voice(max, 2.0);
        assert!(match_voices(&[tiny], &people).is_empty());
    }

    fn status(store: &VoiceStore, label: &str) -> Option<VoiceStatus> {
        store.statuses(&[label.to_string()]).unwrap().remove(label)
    }

    #[test]
    fn nothing_is_learned_while_recognition_is_off() {
        let mut synthetic = SyntheticVoices::new();
        let mut store = VoiceStore::open_in_memory().unwrap();
        let speaker = voice_of(synthetic.person(), 60.0);
        assert!(store.recognize(std::slice::from_ref(&speaker), "r", false).unwrap().is_empty());
        store.link(&speaker.label_id, "Max", false).unwrap();
        assert_eq!(status(&store, &speaker.label_id).unwrap().state, MatchState::Unknown);
        assert!(store.people().unwrap().is_empty());
    }

    #[test]
    fn naming_teaches_a_voice() {
        let mut synthetic = SyntheticVoices::new();
        let mut store = VoiceStore::open_in_memory().unwrap();
        let max = synthetic.person();

        // Meeting 1: unknown, then named.
        let first = voice_of(synthetic.recording(&max, 0.3), 60.0);
        assert!(store.recognize(std::slice::from_ref(&first), "r1", true).unwrap().is_empty());
        assert_eq!(status(&store, &first.label_id).unwrap().state, MatchState::Unknown);
        store.link(&first.label_id, "Max", true).unwrap();
        let people = store.people().unwrap();
        assert_eq!(people.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Max"]);
        assert_eq!(people[0].confirmed_samples, 1);

        // Meetings 2 and 3: guessed (teaches nothing), confirmed by the user.
        for round in 0..2 {
            let next = voice_of(synthetic.recording(&max, 0.3), 60.0);
            let names = store.recognize(std::slice::from_ref(&next), &format!("r{round}"), true).unwrap();
            assert_eq!(names.get(&next.label_id).map(String::as_str), Some("Max"));
            let s = status(&store, &next.label_id).unwrap();
            assert_eq!(s.state, MatchState::Suggested);
            assert_eq!(s.person_name.as_deref(), Some("Max"));
            let person = &store.people().unwrap()[0];
            assert_eq!(person.samples, person.confirmed_samples);
            store.link(&next.label_id, "max", true).unwrap();
        }
        let people = store.people().unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].confirmed_samples, 3);

        // Meeting 4: named automatically.
        let fourth = voice_of(synthetic.recording(&max, 0.3), 60.0);
        let names = store.recognize(std::slice::from_ref(&fourth), "r4", true).unwrap();
        assert_eq!(names.get(&fourth.label_id).map(String::as_str), Some("Max"));
        assert_eq!(status(&store, &fourth.label_id).unwrap().state, MatchState::Automatic);
        let people = store.people().unwrap();
        assert_eq!(people[0].samples, 4);
        assert_eq!(store.named_labels(&people[0].id).unwrap().len(), 4);
    }

    #[test]
    fn correcting_an_automatic_name_moves_the_sample() {
        let mut synthetic = SyntheticVoices::new();
        let mut store = VoiceStore::open_in_memory().unwrap();
        let max = synthetic.person();
        for round in 0..3 {
            let speaker = voice_of(synthetic.recording(&max, 0.3), 60.0);
            store.recognize(std::slice::from_ref(&speaker), &format!("r{round}"), true).unwrap();
            store.link(&speaker.label_id, "Max", true).unwrap();
        }
        let speaker = voice_of(synthetic.recording(&max, 0.3), 60.0);
        let names = store.recognize(std::slice::from_ref(&speaker), "r9", true).unwrap();
        assert_eq!(names.get(&speaker.label_id).map(String::as_str), Some("Max"));
        assert_eq!(store.people().unwrap()[0].samples, 4);

        // It was actually his brother.
        store.link(&speaker.label_id, "Jonas", true).unwrap();
        let by_name: HashMap<String, PersonSummary> =
            store.people().unwrap().into_iter().map(|p| (p.name.clone(), p)).collect();
        assert_eq!(by_name["Max"].samples, 3);
        assert_eq!(by_name["Jonas"].confirmed_samples, 1);

        // A guest: no sample at all.
        store.unlink(&speaker.label_id).unwrap();
        let by_name: HashMap<String, usize> = store.people().unwrap().into_iter().map(|p| (p.name, p.samples)).collect();
        assert_eq!(by_name["Jonas"], 0);
        assert_eq!(status(&store, &speaker.label_id).unwrap().state, MatchState::Unknown);
    }

    #[test]
    fn one_voice_belongs_to_one_person() {
        let mut synthetic = SyntheticVoices::new();
        let mut store = VoiceStore::open_in_memory().unwrap();
        let voice = synthetic.person();
        let first = voice_of(voice.clone(), 60.0);
        let again = voice_of(voice, 60.0);
        store.recognize(std::slice::from_ref(&first), "r1", true).unwrap();
        store.link(&first.label_id, "Max", true).unwrap();
        store.recognize(std::slice::from_ref(&again), "r2", true).unwrap();
        store.link(&again.label_id, "Maximilian", true).unwrap();
        let samples: HashMap<String, usize> = store.people().unwrap().into_iter().map(|p| (p.name, p.samples)).collect();
        assert_eq!(samples["Max"], 0);
        assert_eq!(samples["Maximilian"], 1);
    }

    #[test]
    fn short_speakers_give_no_sample_and_duplicates_one() {
        let mut synthetic = SyntheticVoices::new();
        let mut store = VoiceStore::open_in_memory().unwrap();
        let max = synthetic.person();
        let short = voice_of(max.clone(), 2.0);
        store.recognize(std::slice::from_ref(&short), "r1", true).unwrap();
        store.link(&short.label_id, "Max", true).unwrap();
        assert_eq!(store.people().unwrap()[0].samples, 0);

        let again = [voice_of(max.clone(), 60.0), voice_of(max, 60.0)];
        store.recognize(&again, "r2", true).unwrap();
        for speaker in &again {
            store.link(&speaker.label_id, "Max", true).unwrap();
        }
        assert_eq!(store.people().unwrap()[0].samples, 1);
    }

    #[test]
    fn deleting_removes_the_voice_data() {
        let mut synthetic = SyntheticVoices::new();
        let mut store = VoiceStore::open_in_memory().unwrap();
        let speakers = [voice_of(synthetic.person(), 60.0), voice_of(synthetic.person(), 60.0)];
        store.recognize(&speakers, "rec", true).unwrap();
        store.link(&speakers[0].label_id, "Max", true).unwrap();
        store.link(&speakers[1].label_id, "Anna", true).unwrap();

        let max = store.people().unwrap().into_iter().find(|p| p.name == "Max").unwrap();
        store.delete_person(&max.id).unwrap();
        assert_eq!(store.people().unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Anna"]);
        assert_eq!(status(&store, &speakers[0].label_id).unwrap().state, MatchState::Unknown);

        store.forget_recording("rec").unwrap();
        assert!(status(&store, &speakers[1].label_id).is_none());
        assert_eq!(store.people().unwrap().len(), 1);

        store.delete_all().unwrap();
        assert!(store.people().unwrap().is_empty());
        assert!(!store.has_data().unwrap());
    }

    #[test]
    fn merging_a_dissimilar_voice_teaches_nothing() {
        let mut synthetic = SyntheticVoices::new();
        let mut store = VoiceStore::open_in_memory().unwrap();
        let max = synthetic.person();
        let named = voice_of(synthetic.recording(&max, 0.3), 30.0);
        let similar = voice_of(synthetic.recording(&max, 0.3), 20.0);
        let stranger = voice_of(synthetic.person(), 30.0);
        store.recognize(&[named, similar, stranger], "r", true).unwrap();
        let ids: Vec<String> = store
            .conn
            .prepare("SELECT label_id FROM speaker_voices ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        store.link(&ids[0], "Max", true).unwrap();
        store.merge(&ids[2], &ids[0], true).unwrap();
        assert_eq!(store.people().unwrap()[0].confirmed_samples, 1);
        store.merge(&ids[1], &ids[0], true).unwrap();
        assert_eq!(store.people().unwrap()[0].confirmed_samples, 2);
    }
}
