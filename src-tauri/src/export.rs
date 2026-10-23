//! Renders transcripts as TXT, Markdown, SRT, WebVTT and JSON.
//! A port of Scripta's `TranscriptExporter`.

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::format;
use crate::model::WordTiming;

/// A transcript as plain values, detached from the database.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportDocument {
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub duration: f64,
    pub language: Option<String>,
    pub engine: String,
    pub segments: Vec<ExportSegment>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker: Option<String>,
    pub words: Vec<WordTiming>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Text,
    Markdown,
    Srt,
    Vtt,
    Json,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 5] = [Self::Text, Self::Markdown, Self::Srt, Self::Vtt, Self::Json];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Text => "txt",
            Self::Markdown => "md",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Json => "json",
        }
    }
}

pub fn render(document: &ExportDocument, format: ExportFormat) -> String {
    match format {
        ExportFormat::Text => plain_text(document, true),
        ExportFormat::Markdown => markdown(document),
        ExportFormat::Srt => srt(document),
        ExportFormat::Vtt => vtt(document),
        ExportFormat::Json => json(document),
    }
}

// MARK: - Text

/// Readable text: one paragraph per segment, with timestamps and speakers.
/// Without the header this is what "Transkript kopieren" puts on the clipboard.
pub fn plain_text(document: &ExportDocument, include_header: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    if include_header {
        lines.push(document.title.clone());
        lines.push(header_details(document).join(" · "));
        lines.push(String::new());
    }
    for segment in &document.segments {
        let mut line = format!("[{}] ", format::duration(segment.start));
        if let Some(speaker) = &segment.speaker {
            line.push_str(&format!("{speaker}: "));
        }
        line.push_str(&segment.text);
        lines.push(line);
    }
    lines.join("\n") + "\n"
}

pub fn markdown(document: &ExportDocument) -> String {
    let mut lines = vec![format!("# {}", document.title), String::new()];
    lines.extend(header_details(document).into_iter().map(|d| format!("- {d}")));
    lines.extend(["".into(), "---".into(), "".into()]);
    let mut previous: Option<&String> = None;
    for segment in &document.segments {
        let time = format!("`{}`", format::duration(segment.start));
        match &segment.speaker {
            Some(speaker) if Some(speaker) != previous => {
                lines.push(format!("**{speaker}** {time}  "));
                lines.push(segment.text.clone());
            }
            _ => lines.push(format!("{time} {}", segment.text)),
        }
        lines.push(String::new());
        previous = segment.speaker.as_ref();
    }
    lines.join("\n")
}

fn header_details(document: &ExportDocument) -> Vec<String> {
    let mut details = vec![
        format::long_date_time(document.created_at.with_timezone(&Local)),
        format::duration(document.duration),
    ];
    if let Some(code) = &document.language {
        details.push(format::language_name(code));
    }
    details
}

// MARK: - Subtitles

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker: Option<String>,
}

/// Subtitle-sized cues: at most `max_duration` seconds and `max_characters`
/// characters (two lines). Long segments are split at word boundaries using
/// the word timings.
pub fn cues(document: &ExportDocument, max_duration: f64, max_characters: usize) -> Vec<Cue> {
    let mut cues = Vec::new();
    for segment in &document.segments {
        if segment.words.is_empty() {
            cues.push(Cue { start: segment.start, end: segment.end, text: segment.text.clone(), speaker: segment.speaker.clone() });
            continue;
        }
        let mut current: Vec<&WordTiming> = Vec::new();
        let flush = |current: &mut Vec<&WordTiming>, cues: &mut Vec<Cue>| {
            if let (Some(first), Some(last)) = (current.first(), current.last()) {
                cues.push(Cue {
                    start: first.start,
                    end: last.end,
                    text: current.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
                    speaker: segment.speaker.clone(),
                });
            }
            current.clear();
        };
        for word in &segment.words {
            if let Some(first) = current.first() {
                let length: usize =
                    current.iter().map(|w| w.text.chars().count()).sum::<usize>() + current.len() + word.text.chars().count();
                if length > max_characters || word.end - first.start > max_duration {
                    flush(&mut current, &mut cues);
                }
            }
            current.push(word);
        }
        flush(&mut current, &mut cues);
    }
    cues
}

pub fn srt(document: &ExportDocument) -> String {
    cues(document, 7.0, 84)
        .iter()
        .enumerate()
        .map(|(index, cue)| {
            let text = match &cue.speaker {
                Some(speaker) => format!("{speaker}: {}", cue.text),
                None => cue.text.clone(),
            };
            format!(
                "{}\n{} --> {}\n{}\n",
                index + 1,
                subtitle_time(cue.start, ','),
                subtitle_time(cue.end, ','),
                wrapped(&text, 42)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn vtt(document: &ExportDocument) -> String {
    let body: Vec<String> = cues(document, 7.0, 84)
        .iter()
        .map(|cue| {
            // WebVTT voice tags carry the speaker.
            let text = match &cue.speaker {
                Some(speaker) => format!("<v {speaker}>{}", wrapped(&cue.text, 42)),
                None => wrapped(&cue.text, 42),
            };
            format!("{} --> {}\n{}\n", subtitle_time(cue.start, '.'), subtitle_time(cue.end, '.'), text)
        })
        .collect();
    format!("WEBVTT\n\n{}", body.join("\n"))
}

/// "01:02:03,456" (SRT) or "01:02:03.456" (WebVTT).
pub fn subtitle_time(seconds: f64, separator: char) -> String {
    let total = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        total / 3_600_000,
        total / 60_000 % 60,
        total / 1000 % 60,
        separator,
        total % 1000
    )
}

/// Breaks long cue text into two lines near the middle.
fn wrapped(text: &str, line_length: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= line_length {
        return text.to_string();
    }
    let middle = chars.len() / 2;
    let before = chars[..middle].iter().rposition(|c| *c == ' ');
    let after = chars[middle..].iter().position(|c| *c == ' ').map(|i| i + middle);
    let split = match (before, after) {
        (Some(b), Some(a)) => if middle - b <= a - middle { b } else { a },
        (Some(b), None) => b,
        (None, Some(a)) => a,
        (None, None) => return text.to_string(),
    };
    let first: String = chars[..split].iter().collect();
    let second: String = chars[split + 1..].iter().collect();
    format!("{first}\n{second}")
}

// MARK: - JSON

// Fields in alphabetical order, like Scripta's sorted keys.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonTranscript<'a> {
    created_at: String,
    duration: f64,
    engine: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
    segments: Vec<JsonSegment<'a>>,
    title: &'a str,
}

#[derive(Serialize)]
struct JsonSegment<'a> {
    end: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker: Option<&'a str>,
    start: f64,
    text: &'a str,
    words: Vec<JsonWord<'a>>,
}

#[derive(Serialize)]
struct JsonWord<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<f64>,
    end: f64,
    start: f64,
    text: &'a str,
}

fn ms(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub fn json(document: &ExportDocument) -> String {
    let payload = JsonTranscript {
        created_at: document.created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        duration: ms(document.duration),
        engine: &document.engine,
        language: document.language.as_deref(),
        segments: document
            .segments
            .iter()
            .map(|segment| JsonSegment {
                end: ms(segment.end),
                speaker: segment.speaker.as_deref(),
                start: ms(segment.start),
                text: &segment.text,
                words: segment
                    .words
                    .iter()
                    .map(|word| JsonWord {
                        confidence: word.confidence.map(|c| (c as f64 * 1000.0).round() / 1000.0),
                        end: ms(word.end),
                        start: ms(word.start),
                        text: &word.text,
                    })
                    .collect(),
            })
            .collect(),
        title: &document.title,
    };
    serde_json::to_string_pretty(&payload).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn words(text: &str, start: f64) -> Vec<WordTiming> {
        text.split(' ')
            .enumerate()
            .map(|(i, w)| {
                let begin = start + i as f64 * 0.4;
                WordTiming { text: w.into(), start: begin, end: begin + 0.32, confidence: Some(0.91234) }
            })
            .collect()
    }

    fn document(meeting: bool) -> ExportDocument {
        ExportDocument {
            title: "Team-Weekly".into(),
            created_at: Utc.timestamp_opt(1_760_000_000, 0).unwrap(),
            duration: 3_725.0,
            language: Some("de".into()),
            engine: "parakeet-tdt-0.6b-v3".into(),
            segments: vec![
                ExportSegment {
                    start: 1.2,
                    end: 3.0,
                    text: "Guten Morgen zusammen!".into(),
                    speaker: meeting.then(|| "Andere".into()),
                    words: words("Guten Morgen zusammen!", 1.2),
                },
                ExportSegment {
                    start: 3_661.5,
                    end: 3_663.25,
                    text: "Bis Donnerstag.".into(),
                    speaker: meeting.then(|| "Ich".into()),
                    words: words("Bis Donnerstag.", 3_661.5),
                },
            ],
        }
    }

    #[test]
    fn subtitle_times() {
        assert_eq!(subtitle_time(0.0, ','), "00:00:00,000");
        assert_eq!(subtitle_time(3_661.5, ','), "01:01:01,500");
        assert_eq!(subtitle_time(59.9996, '.'), "00:01:00.000");
    }

    #[test]
    fn srt_cues_have_speakers() {
        let srt = srt(&document(true));
        assert!(srt.starts_with("1\n00:00:01,200 --> 00:00:02,320\nAndere: Guten Morgen zusammen!\n"), "{srt}");
        assert!(srt.contains("\n2\n01:01:01,500 --> 01:01:02,220\nIch: Bis Donnerstag.\n"), "{srt}");
    }

    #[test]
    fn vtt_has_header_and_voice_tags() {
        let vtt = vtt(&document(true));
        assert!(vtt.starts_with("WEBVTT\n\n00:00:01.200 --> "));
        assert!(vtt.contains("<v Andere>Guten Morgen zusammen!"));
        assert!(!super::vtt(&document(false)).contains("<v "));
    }

    #[test]
    fn long_segments_become_short_cues() {
        let sentence = vec!["Wort"; 60].join(" ");
        let long = ExportDocument {
            title: "Lang".into(),
            created_at: Utc::now(),
            duration: 30.0,
            language: None,
            engine: "test".into(),
            segments: vec![ExportSegment { start: 0.0, end: 24.0, text: sentence.clone(), speaker: None, words: words(&sentence, 0.0) }],
        };
        let cues = cues(&long, 7.0, 84);
        assert!(cues.len() > 2);
        assert!(cues.iter().all(|c| c.text.chars().count() <= 84 && c.end - c.start <= 7.01));
        assert_eq!(cues.iter().map(|c| c.text.clone()).collect::<Vec<_>>().join(" "), sentence);
        let srt = srt(&long);
        assert!(srt.lines().filter(|l| !l.contains("-->") && l.parse::<u32>().is_err()).all(|l| l.chars().count() <= 45));
    }

    #[test]
    fn edited_segments_become_one_cue() {
        let edited = ExportDocument {
            title: "x".into(),
            created_at: Utc::now(),
            duration: 5.0,
            language: None,
            engine: "test".into(),
            segments: vec![ExportSegment { start: 1.0, end: 4.0, text: "Korrigierter Text.".into(), speaker: None, words: vec![] }],
        };
        assert_eq!(
            cues(&edited, 7.0, 84),
            [Cue { start: 1.0, end: 4.0, text: "Korrigierter Text.".into(), speaker: None }]
        );
    }

    #[test]
    fn plain_text_has_timestamps_and_speakers() {
        let body = plain_text(&document(true), false);
        assert_eq!(body, "[0:01] Andere: Guten Morgen zusammen!\n[1:01:01] Ich: Bis Donnerstag.\n");
        let full = plain_text(&document(false), true);
        assert!(full.starts_with("Team-Weekly\n"));
        assert!(full.contains("1:02:05"));
        assert!(full.contains("[0:01] Guten Morgen zusammen!"));
    }

    #[test]
    fn markdown_has_title_and_speakers() {
        let markdown = markdown(&document(true));
        assert!(markdown.starts_with("# Team-Weekly\n"));
        assert!(markdown.contains("**Andere** `0:01`  \nGuten Morgen zusammen!"));
        assert!(markdown.contains("**Ich** `1:01:01`"));
    }

    #[test]
    fn json_has_segments_and_rounded_words() {
        let value: serde_json::Value = serde_json::from_str(&json(&document(true))).unwrap();
        assert_eq!(value["title"], "Team-Weekly");
        assert_eq!(value["language"], "de");
        assert_eq!(value["segments"].as_array().unwrap().len(), 2);
        assert_eq!(value["segments"][0]["speaker"], "Andere");
        assert_eq!(value["segments"][0]["words"][0]["text"], "Guten");
        assert_eq!(value["segments"][0]["words"][0]["confidence"], 0.912);
        assert!(value["createdAt"].as_str().unwrap().starts_with("2025-"));
    }

    #[test]
    fn all_formats_produce_text() {
        for format in ExportFormat::ALL {
            assert!(!render(&document(true), format).is_empty());
        }
    }
}
