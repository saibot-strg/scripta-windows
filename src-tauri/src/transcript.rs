//! Turns timed words into readable segments and assigns speakers.
//! A port of Scripta's `TranscriptBuilder` and `ParakeetEngine.words(from:)`;
//! keep both in step.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::model::{SpeakerRole, WordTiming};

/// A speaker within one transcript, before it is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpeakerKey {
    pub role: SpeakerRole,
    /// 1-based for diarized speakers, 0 otherwise.
    pub number: i64,
}

impl SpeakerKey {
    pub const ME: SpeakerKey = SpeakerKey { role: SpeakerRole::Me, number: 0 };
    pub const OTHERS: SpeakerKey = SpeakerKey { role: SpeakerRole::Others, number: 0 };

    pub fn diarized(number: i64) -> SpeakerKey {
        SpeakerKey { role: SpeakerRole::Diarized, number }
    }
}

/// A segment before it is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentDraft {
    pub start: f64,
    pub end: f64,
    pub words: Vec<WordTiming>,
    pub speaker: Option<SpeakerKey>,
}

impl SegmentDraft {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// Segments plus the voice fingerprint of each numbered speaker.
#[derive(Clone, Debug, Default)]
pub struct DraftTranscript {
    pub segments: Vec<SegmentDraft>,
    pub voices: HashMap<SpeakerKey, Vec<f32>>,
}

/// One stretch of speech by one speaker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeakerTurn {
    /// 1-based, numbered in order of first appearance.
    pub speaker: i64,
    pub start: f64,
    pub end: f64,
}

/// Who spoke when in one audio file.
#[derive(Clone, Debug, Default)]
pub struct Diarization {
    pub turns: Vec<SpeakerTurn>,
    /// Voice fingerprint per speaker (L2-normalized), used later to recognize
    /// people across recordings.
    pub embeddings: HashMap<i64, Vec<f32>>,
}

impl Diarization {
    pub fn speakers(&self) -> Vec<i64> {
        self.turns.iter().map(|t| t.speaker).collect::<BTreeSet<_>>().into_iter().collect()
    }
}

pub const DEFAULT_PAUSE: f64 = 1.2;
pub const DEFAULT_MAX_DURATION: f64 = 30.0;

/// A further voice on the meeting microphone with less speech than this is
/// most likely the local user split off or speaker echo, not a person in the
/// room; it stays "Ich".
pub const MINIMUM_VOICE_SPEECH: f64 = 8.0;

fn ends_sentence(word: &str) -> bool {
    matches!(word.chars().last(), Some('.' | '?' | '!' | '…'))
}

/// Splits words into segments: after a sentence ends, at pauses, and at the
/// latest after `max_duration` (long monologues without punctuation).
pub fn segments(words: &[WordTiming], speaker: Option<SpeakerKey>) -> Vec<SegmentDraft> {
    let speakers = vec![speaker; words.len()];
    segments_with(words, &speakers, DEFAULT_PAUSE, DEFAULT_MAX_DURATION)
}

/// Like `segments` with one speaker per word; a change of speaker also ends
/// the segment.
pub fn segments_with(
    words: &[WordTiming],
    speakers: &[Option<SpeakerKey>],
    pause: f64,
    max_duration: f64,
) -> Vec<SegmentDraft> {
    let mut result = Vec::new();
    let mut current: Vec<WordTiming> = Vec::new();
    let mut current_speaker: Option<SpeakerKey> = None;

    fn close(result: &mut Vec<SegmentDraft>, current: &mut Vec<WordTiming>, speaker: Option<SpeakerKey>) {
        if let (Some(first), Some(last)) = (current.first(), current.last()) {
            result.push(SegmentDraft { start: first.start, end: last.end, words: std::mem::take(current), speaker });
        }
    }

    for (word, speaker) in words.iter().zip(speakers.iter()) {
        if let (Some(previous), Some(first)) = (current.last(), current.first()) {
            let is_pause = word.start - previous.end >= pause;
            let is_too_long = word.end - first.start > max_duration;
            if is_pause || is_too_long || *speaker != current_speaker {
                close(&mut result, &mut current, current_speaker);
            }
        }
        current.push(word.clone());
        current_speaker = *speaker;
        if ends_sentence(&word.text) {
            close(&mut result, &mut current, current_speaker);
        }
    }
    close(&mut result, &mut current, current_speaker);
    result
}

/// Merges "Ich" (microphone) and "Andere" (system audio) by time.
///
/// Without headphones the microphone also picks up the other participants
/// from the speakers. Such echo segments repeat what the system track
/// already has at the same time, so they are dropped.
pub fn merge_meeting(me: Vec<SegmentDraft>, others: Vec<SegmentDraft>) -> Vec<SegmentDraft> {
    let mut merged: Vec<SegmentDraft> =
        me.into_iter().filter(|segment| !others.iter().any(|other| is_echo(segment, other))).collect();
    merged.extend(others);
    merged.sort_by(|a, b| (a.start, a.end).partial_cmp(&(b.start, b.end)).unwrap_or(std::cmp::Ordering::Equal));
    merged
}

pub fn is_echo(segment: &SegmentDraft, other: &SegmentDraft) -> bool {
    let overlap = segment.end.min(other.end) - segment.start.max(other.start);
    let length = (segment.end - segment.start).max(0.1);
    if overlap / length < 0.5 {
        return false;
    }
    let mine = normalized_words(&segment.text());
    let theirs = normalized_words(&other.text());
    if mine.is_empty() {
        return false;
    }
    mine.intersection(&theirs).count() as f64 / mine.len() as f64 >= 0.6
}

fn normalized_words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The diarized speaker of each word: the turn overlapping it most, or else
/// the nearest turn. None only without turns.
pub fn speakers_of(words: &[WordTiming], turns: &[SpeakerTurn]) -> Vec<Option<i64>> {
    words
        .iter()
        .map(|word| {
            let mut best: Option<(i64, f64)> = None;
            for turn in turns {
                // Negative overlap is the gap to the turn: nearer is better.
                let overlap = word.end.min(turn.end) - word.start.max(turn.start);
                if let Some((_, best_overlap)) = best {
                    if best_overlap >= overlap {
                        continue;
                    }
                }
                best = Some((turn.speaker, overlap));
            }
            best.map(|(speaker, _)| speaker)
        })
        .collect()
}

/// A single track (microphone recording or import): numbered speakers if
/// diarization found more than one voice, otherwise no speakers.
pub fn transcript(words: &[WordTiming], diarization: Option<&Diarization>) -> DraftTranscript {
    let plain = || DraftTranscript { segments: segments(words, None), voices: HashMap::new() };
    let Some(diarization) = diarization else { return plain() };
    if diarization.speakers().len() <= 1 {
        return plain();
    }
    let keys: Vec<Option<SpeakerKey>> =
        speakers_of(words, &diarization.turns).into_iter().map(|s| s.map(SpeakerKey::diarized)).collect();
    if keys.iter().collect::<HashSet<_>>().len() <= 1 {
        return plain();
    }
    numbered(
        segments_with(words, &keys, DEFAULT_PAUSE, DEFAULT_MAX_DURATION),
        &voices_of(diarization, 0),
    )
}

/// Meeting: every voice on the system track becomes a numbered speaker. On
/// the microphone the main voice stays "Ich"; further voices with enough
/// speech (people in the same room) become speakers as well.
pub fn meeting_transcript(
    microphone: &[WordTiming],
    microphone_speakers: Option<&Diarization>,
    system: &[WordTiming],
    system_speakers: Option<&Diarization>,
) -> DraftTranscript {
    let mut others = segments(system, Some(SpeakerKey::OTHERS));
    let mut system_count = 0;
    let mut all_voices: HashMap<SpeakerKey, Vec<f32>> = HashMap::new();
    if let Some(system_speakers) = system_speakers.filter(|d| !d.turns.is_empty()) {
        let keys: Vec<Option<SpeakerKey>> =
            speakers_of(system, &system_speakers.turns).into_iter().map(|s| s.map(SpeakerKey::diarized)).collect();
        others = segments_with(system, &keys, DEFAULT_PAUSE, DEFAULT_MAX_DURATION);
        system_count = system_speakers.speakers().into_iter().max().unwrap_or(0);
        all_voices = voices_of(system_speakers, 0);
    }

    let Some(microphone_speakers) = microphone_speakers.filter(|d| d.speakers().len() > 1) else {
        return numbered(merge_meeting(segments(microphone, Some(SpeakerKey::ME)), others), &all_voices);
    };
    // Numbered after the system voices; the main voice is chosen after echo
    // removal, so echo of the speakers cannot become "Ich".
    let keys: Vec<Option<SpeakerKey>> = speakers_of(microphone, &microphone_speakers.turns)
        .into_iter()
        .map(|s| s.map(|n| SpeakerKey::diarized(system_count + n)))
        .collect();
    for (key, voice) in voices_of(microphone_speakers, system_count) {
        all_voices.entry(key).or_insert(voice);
    }
    let me = segments_with(microphone, &keys, DEFAULT_PAUSE, DEFAULT_MAX_DURATION);
    let mut merged = merge_meeting(me, others);
    let voices: BTreeSet<SpeakerKey> = keys.iter().flatten().copied().collect();
    if let Some(main) = main_voice(&voices, &merged) {
        let mut mine: HashSet<SpeakerKey> = voices
            .iter()
            .filter(|v| **v != main && speech_of(**v, &merged) < MINIMUM_VOICE_SPEECH)
            .copied()
            .collect();
        mine.insert(main);
        for segment in &mut merged {
            if segment.speaker.is_some_and(|s| mine.contains(&s)) {
                segment.speaker = Some(SpeakerKey::ME);
            }
        }
    }
    numbered(merged, &all_voices)
}

fn voices_of(diarization: &Diarization, offset: i64) -> HashMap<SpeakerKey, Vec<f32>> {
    diarization
        .embeddings
        .iter()
        .map(|(speaker, voice)| (SpeakerKey::diarized(offset + speaker), voice.clone()))
        .collect()
}

/// The voice with the most speech among `voices` (the first on a tie).
fn main_voice(voices: &BTreeSet<SpeakerKey>, segments: &[SegmentDraft]) -> Option<SpeakerKey> {
    let mut best: Option<(SpeakerKey, f64)> = None;
    for voice in voices {
        let speech = speech_of(*voice, segments);
        if best.is_none_or(|(_, b)| speech > b) {
            best = Some((*voice, speech));
        }
    }
    best.map(|(voice, _)| voice)
}

pub fn speech_of(speaker: SpeakerKey, segments: &[SegmentDraft]) -> f64 {
    segments.iter().filter(|s| s.speaker == Some(speaker)).map(|s| s.end - s.start).sum()
}

/// Numbers diarized speakers 1, 2, 3 … in order of first appearance,
/// carrying each speaker's voice along. Voices of speakers no longer in the
/// segments (merged into "Ich") are dropped.
pub fn numbered(segments: Vec<SegmentDraft>, voices: &HashMap<SpeakerKey, Vec<f32>>) -> DraftTranscript {
    let mut numbers: Vec<(i64, i64)> = Vec::new();
    let segments = segments
        .into_iter()
        .map(|mut segment| {
            if let Some(speaker) = segment.speaker.filter(|s| s.role == SpeakerRole::Diarized) {
                let number = match numbers.iter().find(|(old, _)| *old == speaker.number) {
                    Some((_, new)) => *new,
                    None => {
                        let new = numbers.len() as i64 + 1;
                        numbers.push((speaker.number, new));
                        new
                    }
                };
                segment.speaker = Some(SpeakerKey::diarized(number));
            }
            segment
        })
        .collect();
    let mut renumbered = HashMap::new();
    for (old, new) in numbers {
        if let Some(voice) = voices.get(&SpeakerKey::diarized(old)) {
            renumbered.insert(SpeakerKey::diarized(new), voice.clone());
        }
    }
    DraftTranscript { segments, voices: renumbered }
}

/// Speakers with less speech than this are fragments of another voice (a
/// cough, a short "ja", the start of a turn): they join the most similar
/// speaker. Measured: a single person in an 8 s note came out as 3 speakers.
pub const MINIMUM_SPEAKER_SPEECH: f64 = 4.0;
/// Two speakers whose voices are this similar are one person split by the
/// clustering (TitaNet: same person 0.8 and more even over a call, different
/// people at most 0.6 in the test material).
pub const SAME_VOICE_SIMILARITY: f32 = 0.8;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return -1.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norms = (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if norms > 0.0 { dot / norms } else { -1.0 }
}

/// Cleans up the clustering: merges fragments and split voices, then numbers
/// the speakers by first appearance again. Voices of merged speakers are
/// averaged by speech time.
pub fn consolidate(diarization: Diarization) -> Diarization {
    let Diarization { mut turns, mut embeddings } = diarization;
    let speech = |turns: &[SpeakerTurn], speaker: i64| -> f64 {
        turns.iter().filter(|t| t.speaker == speaker).map(|t| t.end - t.start).sum()
    };
    loop {
        let speakers: Vec<i64> = turns.iter().map(|t| t.speaker).collect::<BTreeSet<_>>().into_iter().collect();
        if speakers.len() < 2 {
            break;
        }
        // The pair to merge: a fragment into its most similar speaker, else
        // the two most similar voices if they are one person.
        let smallest = *speakers.iter().min_by(|a, b| speech(&turns, **a).total_cmp(&speech(&turns, **b))).unwrap();
        let similarity = |a: i64, b: i64| match (embeddings.get(&a), embeddings.get(&b)) {
            (Some(x), Some(y)) => cosine(x, y),
            _ => -1.0,
        };
        let merge = if speech(&turns, smallest) < MINIMUM_SPEAKER_SPEECH {
            let target = speakers
                .iter()
                .filter(|s| **s != smallest)
                .max_by(|a, b| {
                    similarity(smallest, **a)
                        .total_cmp(&similarity(smallest, **b))
                        .then(speech(&turns, **a).total_cmp(&speech(&turns, **b)))
                })
                .copied();
            target.map(|target| (smallest, target))
        } else {
            let mut best: Option<(i64, i64, f32)> = None;
            for (i, a) in speakers.iter().enumerate() {
                for b in &speakers[i + 1..] {
                    let value = similarity(*a, *b);
                    if value >= SAME_VOICE_SIMILARITY && best.is_none_or(|(_, _, v)| value > v) {
                        best = Some((*a, *b, value));
                    }
                }
            }
            // Keep the speaker with more speech.
            best.map(|(a, b, _)| if speech(&turns, a) < speech(&turns, b) { (a, b) } else { (b, a) })
        };
        let Some((from, into)) = merge else { break };
        let (from_speech, into_speech) = (speech(&turns, from), speech(&turns, into));
        if let (Some(a), Some(b)) = (embeddings.remove(&from), embeddings.get(&into).cloned()) {
            let total = (from_speech + into_speech).max(1e-6);
            let mixed: Vec<f32> = a
                .iter()
                .zip(&b)
                .map(|(x, y)| (*x as f64 * from_speech / total + *y as f64 * into_speech / total) as f32)
                .collect();
            let norm = mixed.iter().map(|x| x * x).sum::<f32>().sqrt();
            embeddings.insert(into, if norm > 0.0 { mixed.iter().map(|x| x / norm).collect() } else { mixed });
        }
        for turn in &mut turns {
            if turn.speaker == from {
                turn.speaker = into;
            }
        }
    }
    // Number by first appearance again.
    turns.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut numbers: Vec<(i64, i64)> = Vec::new();
    for turn in &mut turns {
        let number = match numbers.iter().find(|(old, _)| *old == turn.speaker) {
            Some((_, new)) => *new,
            None => {
                let new = numbers.len() as i64 + 1;
                numbers.push((turn.speaker, new));
                new
            }
        };
        turn.speaker = number;
    }
    let embeddings = numbers.iter().filter_map(|(old, new)| embeddings.get(old).map(|e| (*new, e.clone()))).collect();
    Diarization { turns, embeddings }
}

/// One sub-word token of the recognizer with its timing.
#[derive(Clone, Debug)]
pub struct TokenTiming {
    pub token: String,
    pub start: f64,
    pub end: f64,
    pub confidence: Option<f32>,
}

/// Joins sub-word tokens into words. A token starting with "▁" (or a space)
/// begins a new word; confidence is the mean over its tokens.
pub fn words_from_tokens(tokens: &[TokenTiming]) -> Vec<WordTiming> {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut start = 0.0;
    let mut end = 0.0;
    let mut confidences: Vec<f32> = Vec::new();

    fn flush(words: &mut Vec<WordTiming>, text: &mut String, start: f64, end: f64, confidences: &mut Vec<f32>) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let confidence =
                (!confidences.is_empty()).then(|| confidences.iter().sum::<f32>() / confidences.len() as f32);
            words.push(WordTiming { text: trimmed.to_string(), start, end, confidence });
        }
        text.clear();
        confidences.clear();
    }

    for token in tokens {
        let piece = token.token.as_str();
        if piece.is_empty() || piece == "<blank>" || piece == "<pad>" || piece == "<unk>" {
            continue;
        }
        let starts_word = piece.starts_with('▁') || piece.starts_with(' ');
        if starts_word || text.is_empty() {
            flush(&mut words, &mut text, start, end, &mut confidences);
            text = piece.trim_start_matches(['▁', ' ']).to_string();
            start = token.start;
        } else {
            text.push_str(piece);
        }
        end = token.end;
        if let Some(confidence) = token.confidence {
            confidences.push(confidence);
        }
    }
    flush(&mut words, &mut text, start, end, &mut confidences);
    words
}

/// Dominant language (BCP-47) of the text. Parakeet detects the language
/// itself but does not report it.
pub fn detect_language(text: &str) -> Option<String> {
    use whatlang::Lang::*;
    let info = whatlang::detect(text)?;
    let code = match info.lang() {
        Deu => "de",
        Eng => "en",
        Fra => "fr",
        Spa => "es",
        Ita => "it",
        Nld => "nl",
        Por => "pt",
        Pol => "pl",
        Ces => "cs",
        Slk => "sk",
        Swe => "sv",
        Dan => "da",
        Nob => "nb",
        Fin => "fi",
        Hun => "hu",
        Ron => "ro",
        Bul => "bg",
        Hrv => "hr",
        Slv => "sl",
        Est => "et",
        Lav => "lv",
        Lit => "lt",
        Ell => "el",
        Ukr => "uk",
        Rus => "ru",
        other => return Some(other.code().to_string()),
    };
    Some(code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str, start: f64) -> Vec<WordTiming> {
        let step = 0.4;
        text.split(' ')
            .enumerate()
            .map(|(index, word)| {
                let begin = start + index as f64 * step;
                WordTiming { text: word.into(), start: begin, end: begin + step * 0.8, confidence: Some(0.9) }
            })
            .collect()
    }

    fn texts(segments: &[SegmentDraft]) -> Vec<String> {
        segments.iter().map(SegmentDraft::text).collect()
    }

    fn turn(speaker: i64, start: f64, end: f64) -> SpeakerTurn {
        SpeakerTurn { speaker, start, end }
    }

    fn diarization(turns: Vec<SpeakerTurn>) -> Diarization {
        Diarization { turns, embeddings: HashMap::new() }
    }

    #[test]
    fn sentences_become_segments() {
        let result = segments(&words("Hallo zusammen. Wie geht es euch? Gut!", 0.0), None);
        assert_eq!(texts(&result), ["Hallo zusammen.", "Wie geht es euch?", "Gut!"]);
        assert_eq!(result[1].start, 0.8);
    }

    #[test]
    fn long_pauses_split_segments() {
        let mut all = words("erster Gedanke ohne Punkt", 0.0);
        all.extend(words("zweiter Gedanke", 5.0));
        let result = segments(&all, None);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].text(), "zweiter Gedanke");
    }

    #[test]
    fn monologues_are_cut_at_the_maximum_duration() {
        let long = words(&vec!["wort"; 100].join(" "), 0.0);
        let result = segments_with(&long, &vec![None; 100], DEFAULT_PAUSE, 10.0);
        assert!(result.len() >= 4);
        assert!(result.iter().all(|s| s.end - s.start <= 10.0));
        assert_eq!(result.iter().map(|s| s.words.len()).sum::<usize>(), 100);
    }

    #[test]
    fn tokens_are_joined_into_words() {
        let token = |t: &str, s: f64, e: f64, c: f32| TokenTiming { token: t.into(), start: s, end: e, confidence: Some(c) };
        let tokens = [
            token("▁Gu", 0.0, 0.2, 0.8),
            token("ten", 0.2, 0.4, 1.0),
            token("▁Mor", 0.5, 0.7, 0.9),
            token("gen", 0.7, 0.9, 0.9),
            token(".", 0.9, 1.0, 0.9),
            token("<blank>", 1.0, 1.1, 0.1),
        ];
        let result = words_from_tokens(&tokens);
        assert_eq!(result.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Guten", "Morgen."]);
        assert_eq!((result[0].start, result[0].end), (0.0, 0.4));
        assert!((result[0].confidence.unwrap() - 0.9).abs() < 0.001);
        assert_eq!(result[1].end, 1.0);
    }

    #[test]
    fn meeting_merge_drops_echo() {
        let others = vec![SegmentDraft {
            start: 1.0,
            end: 3.0,
            words: words("Wie lange brauchst du?", 1.0),
            speaker: Some(SpeakerKey::OTHERS),
        }];
        let me = vec![
            SegmentDraft { start: 1.1, end: 3.0, words: words("wie lange brauchst du", 1.1), speaker: Some(SpeakerKey::ME) },
            SegmentDraft { start: 3.5, end: 5.0, words: words("Bis Donnerstag.", 3.5), speaker: Some(SpeakerKey::ME) },
        ];
        let merged = merge_meeting(me, others);
        assert_eq!(merged.iter().map(|s| s.speaker).collect::<Vec<_>>(), [Some(SpeakerKey::OTHERS), Some(SpeakerKey::ME)]);
        assert_eq!(merged.last().unwrap().text(), "Bis Donnerstag.");
    }

    #[test]
    fn overlapping_speech_is_not_echo() {
        let other = SegmentDraft { start: 0.0, end: 2.0, words: words("Das Budget ist knapp", 0.0), speaker: Some(SpeakerKey::OTHERS) };
        let mine = SegmentDraft { start: 0.5, end: 2.0, words: words("Ja genau so sehe ich das", 0.5), speaker: Some(SpeakerKey::ME) };
        assert!(!is_echo(&mine, &other));
    }

    #[test]
    fn words_go_to_the_turn_they_overlap_most() {
        let turns = [turn(1, 0.0, 1.0), turn(2, 1.1, 3.0)];
        let spoken = [
            WordTiming::new("a", 0.1, 0.4),
            WordTiming::new("b", 0.7, 1.2),
            WordTiming::new("c", 0.9, 1.5),
            WordTiming::new("d", 4.0, 4.2),
        ];
        assert_eq!(speakers_of(&spoken, &turns), [Some(1), Some(1), Some(2), Some(2)]);
        assert_eq!(speakers_of(&spoken, &[]), [None, None, None, None]);
    }

    #[test]
    fn a_change_of_speaker_ends_the_segment() {
        let spoken = words("Ja genau das machen wir so", 0.0);
        let d = |n| Some(SpeakerKey::diarized(n));
        let keys = [d(1), d(1), d(2), d(2), d(2), d(1)];
        let result = segments_with(&spoken, &keys, DEFAULT_PAUSE, DEFAULT_MAX_DURATION);
        assert_eq!(texts(&result), ["Ja genau", "das machen wir", "so"]);
        assert_eq!(result.iter().map(|s| s.speaker).collect::<Vec<_>>(), [d(1), d(2), d(1)]);
    }

    #[test]
    fn single_recordings_get_speakers_only_with_more_than_one_voice() {
        let spoken = words("Hallo zusammen. Hallo Anna.", 0.0);
        let one = diarization(vec![turn(1, 0.0, 2.0)]);
        assert!(transcript(&spoken, Some(&one)).segments.iter().all(|s| s.speaker.is_none()));
        assert!(transcript(&spoken, None).segments.iter().all(|s| s.speaker.is_none()));

        // Diarization numbers may differ from first appearance in the words.
        let two = diarization(vec![turn(2, 0.0, 0.75), turn(1, 0.75, 2.0)]);
        let result = transcript(&spoken, Some(&two)).segments;
        assert_eq!(texts(&result), ["Hallo zusammen.", "Hallo Anna."]);
        assert_eq!(
            result.iter().map(|s| s.speaker).collect::<Vec<_>>(),
            [Some(SpeakerKey::diarized(1)), Some(SpeakerKey::diarized(2))]
        );
    }

    #[test]
    fn meetings_number_remote_voices_and_keep_me() {
        let system = words("Wie weit seid ihr? Fast fertig.", 0.0);
        let system_speakers = diarization(vec![turn(1, 0.0, 1.5), turn(2, 1.55, 2.5)]);
        let microphone = words("Dann bis Freitag.", 4.0);
        let mic_speakers = diarization(vec![turn(1, 4.0, 5.5)]);
        let result = meeting_transcript(&microphone, Some(&mic_speakers), &system, Some(&system_speakers)).segments;
        assert_eq!(
            result.iter().map(|s| s.speaker).collect::<Vec<_>>(),
            [Some(SpeakerKey::diarized(1)), Some(SpeakerKey::diarized(2)), Some(SpeakerKey::ME)]
        );

        let plain = meeting_transcript(&microphone, None, &system, None).segments;
        let speakers: HashSet<_> = plain.iter().filter_map(|s| s.speaker).collect();
        assert_eq!(speakers, HashSet::from([SpeakerKey::OTHERS, SpeakerKey::ME]));
    }

    #[test]
    fn a_person_in_the_room_gets_a_number_short_voices_stay_me() {
        let mut microphone = words(&vec!["ich"; 30].join(" "), 0.0);
        microphone.extend(words(&vec!["kollege"; 25].join(" "), 20.0));
        microphone.extend(words("hm ja", 40.0));
        let mic_speakers = diarization(vec![turn(1, 0.0, 12.0), turn(2, 20.0, 30.0), turn(3, 40.0, 41.0)]);
        let system = words("Klingt gut.", 50.0);
        let system_speakers = diarization(vec![turn(1, 50.0, 51.0)]);
        let result = meeting_transcript(&microphone, Some(&mic_speakers), &system, Some(&system_speakers)).segments;
        let speaker_of_word: Vec<(String, Option<SpeakerKey>)> =
            result.iter().flat_map(|s| s.words.iter().map(move |w| (w.text.clone(), s.speaker))).collect();
        assert!(speaker_of_word.iter().filter(|(w, _)| w == "ich").all(|(_, s)| *s == Some(SpeakerKey::ME)));
        assert!(speaker_of_word.iter().filter(|(w, _)| w == "kollege").all(|(_, s)| *s == Some(SpeakerKey::diarized(1))));
        assert!(speaker_of_word.iter().filter(|(w, _)| w == "hm" || w == "ja").all(|(_, s)| *s == Some(SpeakerKey::ME)));
        assert_eq!(result.last().unwrap().speaker, Some(SpeakerKey::diarized(2)));
    }

    #[test]
    fn voices_follow_the_renumbering() {
        let spoken = words("Hallo zusammen. Hallo Anna.", 0.0);
        let mut two = diarization(vec![turn(2, 0.0, 0.75), turn(1, 0.75, 2.0)]);
        two.embeddings.insert(2, vec![1.0, 0.0]);
        two.embeddings.insert(1, vec![0.0, 1.0]);
        let result = transcript(&spoken, Some(&two));
        assert_eq!(result.voices[&SpeakerKey::diarized(1)], vec![1.0, 0.0]);
        assert_eq!(result.voices[&SpeakerKey::diarized(2)], vec![0.0, 1.0]);
    }

    #[test]
    fn fragments_and_split_voices_are_merged() {
        let a = vec![1.0, 0.0, 0.0];
        let a2 = vec![0.95, 0.31, 0.0]; // the same person, split off (cos ≈ 0.95)
        let b = vec![0.0, 1.0, 0.0];
        let fragment = vec![0.1, 0.0, 1.0]; // 1.5 s, closest to a
        let mut embeddings = HashMap::new();
        embeddings.insert(7, a.clone());
        embeddings.insert(3, fragment);
        embeddings.insert(5, b);
        embeddings.insert(9, a2);
        let d = Diarization {
            turns: vec![turn(7, 0.0, 10.0), turn(3, 10.5, 12.0), turn(5, 13.0, 25.0), turn(9, 26.0, 36.0), turn(7, 37.0, 40.0)],
            embeddings,
        };
        let result = consolidate(d);
        assert_eq!(result.speakers(), [1, 2]);
        assert_eq!(result.turns.iter().map(|t| t.speaker).collect::<Vec<_>>(), [1, 1, 2, 1, 1]);
        assert_eq!(result.embeddings.len(), 2);
    }

    #[test]
    fn distinct_voices_stay_apart() {
        let mut embeddings = HashMap::new();
        embeddings.insert(1, vec![1.0, 0.0]);
        embeddings.insert(2, vec![0.5, 0.866]); // cos 0.5
        let d = Diarization { turns: vec![turn(1, 0.0, 6.0), turn(2, 7.0, 13.0)], embeddings };
        assert_eq!(consolidate(d).speakers(), [1, 2]);
    }

    #[test]
    fn language_detection() {
        assert_eq!(detect_language("Wir besprechen heute das Budget für das nächste Quartal.").as_deref(), Some("de"));
        assert_eq!(detect_language("We review the product roadmap for the next quarter today.").as_deref(), Some("en"));
    }
}
