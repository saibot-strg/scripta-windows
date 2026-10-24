//! The speech pipeline with the real models and synthetic voices (Scripta's
//! test audio: macOS voices, a dialogue of Samantha and Daniel, and the same
//! voices band-limited and compressed like a Teams call).
//!
//! Skipped unless the models are installed (`cargo run --example models`).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use scripta_lib::audio::{mf, MODEL_RATE};
use scripta_lib::engine::{SpeakerEngine, SpeechEngine};
use scripta_lib::gpu::Device;
use scripta_lib::models::ModelKind;
use scripta_lib::transcript::{self, SpeakerKey};
use scripta_lib::voices::{LabelVoice, MatchState, VoiceStore, AUTOMATIC_THRESHOLD};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn samples(name: &str) -> Vec<f32> {
    mf::decode_to_vec(&fixture(name), MODEL_RATE, |_| {}).expect("decode fixture")
}

/// The processor, or a graphics card with SCRIPTA_GPU (see `Device::from_env`).
fn device() -> Device {
    Device::from_env()
}

fn models_installed() -> bool {
    let installed = ModelKind::Speech.is_installed() && ModelKind::Speakers.is_installed();
    if !installed {
        eprintln!("models not installed, skipping (cargo run --example models)");
    }
    installed
}

fn text_of(words: &[scripta_lib::model::WordTiming]) -> String {
    words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Start and end of each turn in `speakers-en.m4a`.
const TURNS: [(f64, f64); 6] = [(0.0, 5.84), (6.54, 12.94), (13.64, 19.78), (20.48, 26.38), (27.08, 33.23), (33.93, 39.62)];

#[test]
fn german_speech() {
    if !models_installed() {
        return;
    }
    let engine = SpeechEngine::load(device()).unwrap();
    let audio = samples("speech-de.m4a");
    let words = engine.transcribe(&audio, &AtomicBool::new(false), |_| {}).unwrap();
    let text = text_of(&words);
    println!("{text}");
    for expected in ["guten morgen", "budget", "quartal", "freitag"] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    assert_eq!(transcript::detect_language(&text).as_deref(), Some("de"));
    // Timings are monotonic and inside the file.
    assert!(words.windows(2).all(|w| w[0].start <= w[1].start));
    assert!(words.last().unwrap().end <= audio.len() as f64 / MODEL_RATE as f64 + 0.5);
}

#[test]
fn english_speech() {
    if !models_installed() {
        return;
    }
    let engine = SpeechEngine::load(device()).unwrap();
    let words = engine.transcribe(&samples("speech-en.m4a"), &AtomicBool::new(false), |_| {}).unwrap();
    let text = text_of(&words);
    println!("{text}");
    assert!(text.contains("good morning"), "{text}");
    assert!(text.contains("roadmap") || text.contains("road map"), "{text}");
    assert!(text.contains("friday"), "{text}");
    assert_eq!(transcript::detect_language(&text).as_deref(), Some("en"));
}

#[test]
fn two_alternating_voices_are_told_apart() {
    if !models_installed() {
        return;
    }
    let engine = SpeakerEngine::load(device()).unwrap();
    let diarization = engine.diarize(&samples("speakers-en.m4a"), None).unwrap();
    println!("{:?}", diarization.turns);
    assert_eq!(diarization.speakers(), [1, 2]);
    assert_eq!(diarization.embeddings.len(), 2);
    assert_eq!(diarization.embeddings[&1].len(), 192);
    let speakers: Vec<Option<i64>> = TURNS
        .iter()
        .map(|(start, end)| {
            let middle = (start + end) / 2.0;
            diarization.turns.iter().find(|t| t.start <= middle && middle <= t.end).map(|t| t.speaker)
        })
        .collect();
    assert_eq!(speakers, [Some(1), Some(2), Some(1), Some(2), Some(1), Some(2)], "{:?}", diarization.turns);
}

#[test]
fn a_fixed_number_of_speakers_is_respected() {
    if !models_installed() {
        return;
    }
    let engine = SpeakerEngine::load(device()).unwrap();
    let audio = samples("speakers-en.m4a");
    assert_eq!(engine.diarize(&audio, Some(1)).unwrap().speakers(), [1]);
    assert_eq!(engine.diarize(&audio, Some(3)).unwrap().speakers().len(), 3);
}

#[test]
fn each_sentence_is_attributed_to_its_speaker() {
    if !models_installed() {
        return;
    }
    let audio = samples("speakers-en.m4a");
    let diarization = SpeakerEngine::load(device()).unwrap().diarize(&audio, None).unwrap();
    let words = SpeechEngine::load(device()).unwrap().transcribe(&audio, &AtomicBool::new(false), |_| {}).unwrap();
    let draft = transcript::transcript(&words, Some(&diarization));
    for segment in &draft.segments {
        println!("{:?}: {}", segment.speaker, segment.text());
    }
    let speaker = |phrase: &str| draft.segments.iter().find(|s| s.text().to_lowercase().contains(phrase)).and_then(|s| s.speaker);
    assert_eq!(speaker("release plan"), Some(SpeakerKey::diarized(1)));
    assert_eq!(speaker("budget"), Some(SpeakerKey::diarized(2)));
    assert_eq!(speaker("finance"), Some(SpeakerKey::diarized(1)));
    assert_eq!(speaker("migration"), Some(SpeakerKey::diarized(2)));
    assert_eq!(speaker("friday"), Some(SpeakerKey::diarized(1)));
    // Not "fallback": onnxruntime 1.24 (the DirectML build) hears "fullback".
    assert_eq!(speaker("delivery"), Some(SpeakerKey::diarized(2)));
}

#[test]
fn named_voices_are_suggested_again_over_a_call() {
    if !models_installed() {
        return;
    }
    let engine = SpeakerEngine::load(device()).unwrap();
    let first = engine.diarize(&samples("speakers-en.m4a"), None).unwrap();
    let second = engine.diarize(&samples("speakers-en-teams.m4a"), None).unwrap();
    let mut store = VoiceStore::open_in_memory().unwrap();
    let voices = |d: &transcript::Diarization, prefix: &str| -> Vec<LabelVoice> {
        d.speakers()
            .into_iter()
            .map(|speaker| LabelVoice {
                label_id: format!("{prefix}{speaker}"),
                embedding: d.embeddings.get(&speaker).cloned().unwrap_or_default(),
                speech_seconds: d.turns.iter().filter(|t| t.speaker == speaker).map(|t| t.end - t.start).sum(),
            })
            .collect()
    };
    store.recognize(&voices(&first, "a"), "first", true).unwrap();
    store.link("a1", "Samantha", true).unwrap();
    store.link("a2", "Daniel", true).unwrap();

    // Daniel speaks first in the second recording.
    store.recognize(&voices(&second, "b"), "second", true).unwrap();
    let statuses = store.statuses(&["b1".into(), "b2".into()]).unwrap();
    println!("{statuses:?}");
    assert_eq!(statuses["b1"].person_name.as_deref(), Some("Daniel"));
    assert_eq!(statuses["b2"].person_name.as_deref(), Some("Samantha"));
    assert_eq!(statuses["b1"].state, MatchState::Suggested);
    assert!(statuses["b1"].score.unwrap() >= AUTOMATIC_THRESHOLD as f64, "{:?}", statuses["b1"].score);
}

/// Turns of `meeting-de.m4a` (Windows voices Hedda, Stefan, Katja; made with
/// scripts/make-test-voices.ps1 and scripts/make-test-meetings.py).
const MEETING_DE: [(f64, f64, &str); 9] = [
    (0.00, 9.18, "hedda"),
    (9.88, 19.02, "stefan"),
    (19.72, 28.89, "katja"),
    (29.59, 36.98, "hedda"),
    (37.68, 44.48, "stefan"),
    (45.18, 52.20, "katja"),
    (52.90, 62.36, "hedda"),
    (63.06, 72.92, "katja"),
    (73.62, 78.88, "stefan"),
];

/// Turns of `meeting-de-call.m4a`: the same voices, other sentences, sounding
/// like a call (band-limited, noise, 8-bit μ-law).
const MEETING_DE_CALL: [(f64, f64, &str); 6] = [
    (0.00, 9.04, "stefan"),
    (9.74, 18.16, "hedda"),
    (18.86, 26.24, "katja"),
    (26.94, 34.79, "stefan"),
    (35.49, 44.69, "hedda"),
    (45.39, 50.41, "katja"),
];

/// The diarized speaker overlapping each turn most.
fn speakers_of_turns(diarization: &transcript::Diarization, turns: &[(f64, f64, &str)]) -> Vec<Option<i64>> {
    turns
        .iter()
        .map(|(start, end, _)| {
            let mut overlap = std::collections::HashMap::<i64, f64>::new();
            for t in &diarization.turns {
                let shared = end.min(t.end) - start.max(t.start);
                if shared > 0.0 {
                    *overlap.entry(t.speaker).or_default() += shared;
                }
            }
            overlap.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(s, _)| s)
        })
        .collect()
}

/// Every person maps to exactly one speaker and vice versa.
fn assert_consistent(found: &[Option<i64>], turns: &[(f64, f64, &str)]) -> std::collections::HashMap<String, i64> {
    let mut mapping = std::collections::HashMap::<String, i64>::new();
    for (label, (_, _, who)) in found.iter().zip(turns) {
        let label = label.expect("every turn has a speaker");
        let known = *mapping.entry(who.to_string()).or_insert(label);
        assert_eq!(known, label, "{who} split across speakers: {found:?}");
    }
    let distinct: std::collections::HashSet<_> = mapping.values().collect();
    assert_eq!(distinct.len(), mapping.len(), "two people merged: {mapping:?}");
    mapping
}

#[test]
fn a_german_meeting_with_three_people_is_separated() {
    if !models_installed() {
        return;
    }
    let engine = SpeakerEngine::load(device()).unwrap();
    for (file, turns) in [("meeting-de.m4a", &MEETING_DE[..]), ("meeting-de-call.m4a", &MEETING_DE_CALL[..])] {
        let diarization = engine.diarize(&samples(file), None).unwrap();
        assert_eq!(diarization.speakers().len(), 3, "{file}: {:?}", diarization.turns);
        assert_consistent(&speakers_of_turns(&diarization, turns), turns);
    }
}

#[test]
fn german_meeting_words_go_to_the_right_people() {
    if !models_installed() {
        return;
    }
    let audio = samples("meeting-de.m4a");
    let diarization = SpeakerEngine::load(device()).unwrap().diarize(&audio, None).unwrap();
    let words = SpeechEngine::load(device()).unwrap().transcribe(&audio, &AtomicBool::new(false), |_| {}).unwrap();
    let draft = transcript::transcript(&words, Some(&diarization));
    for segment in &draft.segments {
        println!("{:?}: {}", segment.speaker.map(|s| s.number), segment.text());
    }
    let speaker = |phrase: &str| {
        draft.segments.iter().find(|s| s.text().to_lowercase().contains(phrase)).and_then(|s| s.speaker)
    };
    let hedda = speaker("quartal");
    let stefan = speaker("betriebsrat");
    let katja = speaker("buchhaltung");
    println!("{hedda:?} {stefan:?} {katja:?}");
    assert!(hedda.is_some() && stefan.is_some() && katja.is_some());
    assert_ne!(hedda, stefan);
    assert_ne!(hedda, katja);
    assert_ne!(stefan, katja);
    assert_eq!(speaker("zeitplan"), hedda);
    assert_eq!(speaker("lieferung"), katja);
    // The last words of the file are not cut off.
    assert!(draft.segments.last().unwrap().text().to_lowercase().contains("nächste woche"), "{:?}", draft.segments.last());
}

#[test]
fn named_colleagues_are_recognized_in_the_next_call() {
    if !models_installed() {
        return;
    }
    let engine = SpeakerEngine::load(device()).unwrap();
    let first = engine.diarize(&samples("meeting-de.m4a"), None).unwrap();
    let second = engine.diarize(&samples("meeting-de-call.m4a"), None).unwrap();
    let first_people = assert_consistent(&speakers_of_turns(&first, &MEETING_DE), &MEETING_DE);
    let second_people = assert_consistent(&speakers_of_turns(&second, &MEETING_DE_CALL), &MEETING_DE_CALL);
    let voices = |d: &transcript::Diarization, prefix: &str| -> Vec<LabelVoice> {
        d.speakers()
            .into_iter()
            .map(|speaker| LabelVoice {
                label_id: format!("{prefix}{speaker}"),
                embedding: d.embeddings.get(&speaker).cloned().unwrap_or_default(),
                speech_seconds: d.turns.iter().filter(|t| t.speaker == speaker).map(|t| t.end - t.start).sum(),
            })
            .collect()
    };
    let mut store = VoiceStore::open_in_memory().unwrap();
    store.recognize(&voices(&first, "a"), "first", true).unwrap();
    for (name, speaker) in &first_people {
        store.link(&format!("a{speaker}"), name, true).unwrap();
    }
    let names = store.recognize(&voices(&second, "b"), "second", true).unwrap();
    let statuses = store.statuses(&second_people.values().map(|s| format!("b{s}")).collect::<Vec<_>>()).unwrap();
    println!("{statuses:?}");
    for (name, speaker) in &second_people {
        assert_eq!(names.get(&format!("b{speaker}")), Some(name), "{statuses:?}");
    }
}
