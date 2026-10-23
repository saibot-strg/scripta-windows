//! Compares speaker embedding models and clustering thresholds on synthetic
//! dialogues (development tool).
//!
//! cargo run --release --example speaker_lab -- <clips dir> <model.onnx>…
//! Clips are named `<session>-<index>-<speaker>.wav` (see tts.ps1).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use scripta_lib::audio::{resample::resample, wav, MODEL_RATE};
use scripta_lib::engine::SpeakerEngine;
use scripta_lib::voices::cosine;

struct Clip {
    speaker: String,
    samples: Vec<f32>,
}

fn load_clips(dir: &Path, session: &str) -> Vec<Clip> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(&format!("{session}-")))
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let mut samples = Vec::new();
            let rate = wav::read_chunks(path, |chunk| {
                samples.extend_from_slice(chunk);
                Ok(())
            })
            .unwrap();
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            Clip { speaker: name.rsplit('-').next().unwrap().to_string(), samples: resample(&samples, rate, MODEL_RATE) }
        })
        .collect()
}

/// Like a call: band-limited to ~5.5 kHz, rumble removed, a little noise,
/// 8-bit μ-law quantization.
fn call_like(samples: &[f32]) -> Vec<f32> {
    let narrow = resample(&resample(samples, MODEL_RATE, 11_025), 11_025, MODEL_RATE);
    let mut state = 0x1234_5678u32;
    let mut previous_in = 0.0f32;
    let mut previous_out = 0.0f32;
    narrow
        .iter()
        .map(|x| {
            // One-pole high-pass at ~150 Hz.
            let high = 0.943 * (previous_out + x - previous_in);
            previous_in = *x;
            previous_out = high;
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32 - 0.5) * 0.01;
            let value = (high * 0.8 + noise).clamp(-1.0, 1.0);
            let mu = 255.0f32;
            let encoded = value.signum() * (1.0 + mu * value.abs()).ln() / (1.0 + mu).ln();
            let quantized = (encoded * 127.0).round() / 127.0;
            quantized.signum() * ((1.0 + mu).powf(quantized.abs()) - 1.0) / mu
        })
        .collect()
}

/// The clips one after another with 0.7 s of silence; returns the turns.
fn dialogue(clips: &[Clip]) -> (Vec<f32>, Vec<(f64, f64, String)>) {
    let mut audio = Vec::new();
    let mut turns = Vec::new();
    for clip in clips {
        let start = audio.len() as f64 / MODEL_RATE as f64;
        audio.extend_from_slice(&clip.samples);
        turns.push((start, audio.len() as f64 / MODEL_RATE as f64, clip.speaker.clone()));
        audio.extend(std::iter::repeat_n(0.0, (MODEL_RATE as f64 * 0.7) as usize));
    }
    (audio, turns)
}

fn mean(vectors: &[Vec<f32>]) -> Vec<f32> {
    let mut sum = vec![0.0; vectors[0].len()];
    for v in vectors {
        sum.iter_mut().zip(v).for_each(|(s, x)| *s += x);
    }
    let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    sum.into_iter().map(|x| x / norm).collect()
}

/// Fraction of turns whose speaker label is consistent with one person.
fn turn_accuracy(d: &scripta_lib::transcript::Diarization, turns: &[(f64, f64, String)]) -> (usize, f64) {
    // The found speaker overlapping each true turn most.
    let found: Vec<Option<i64>> = turns
        .iter()
        .map(|(start, end, _)| {
            let mut overlap: HashMap<i64, f64> = HashMap::new();
            for t in &d.turns {
                let shared = end.min(t.end) - start.max(t.start);
                if shared > 0.0 {
                    *overlap.entry(t.speaker).or_default() += shared;
                }
            }
            overlap.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(s, _)| s)
        })
        .collect();
    // Majority label per true speaker.
    let mut votes: HashMap<&str, HashMap<i64, usize>> = HashMap::new();
    for ((_, _, who), label) in turns.iter().zip(&found) {
        if let Some(label) = label {
            *votes.entry(who.as_str()).or_default().entry(*label).or_default() += 1;
        }
    }
    let majority: HashMap<&str, i64> =
        votes.iter().map(|(who, counts)| (*who, *counts.iter().max_by_key(|(_, c)| **c).unwrap().0)).collect();
    let correct = turns.iter().zip(&found).filter(|((_, _, who), label)| label.is_some() && majority.get(who.as_str()) == label.as_ref()).count();
    let distinct: std::collections::HashSet<i64> = majority.values().copied().collect();
    (d.speakers().len(), if distinct.len() == majority.len() { correct as f64 / turns.len() as f64 } else { 0.0 })
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(&args[0]);
    let first = load_clips(&dir, "a");
    let second = load_clips(&dir, "b");
    let (dialogue_a, turns_a) = dialogue(&first);
    let call_clips: Vec<Clip> = second.iter().map(|c| Clip { speaker: c.speaker.clone(), samples: call_like(&c.samples) }).collect();
    let (dialogue_b, turns_b) = dialogue(&call_clips);
    wav::write_mono(&dir.join("dialogue-a.wav"), &dialogue_a, MODEL_RATE)?;
    wav::write_mono(&dir.join("dialogue-b-call.wav"), &dialogue_b, MODEL_RATE)?;
    println!("dialogue a: {:.1} s, b (call): {:.1} s", dialogue_a.len() as f64 / 16000.0, dialogue_b.len() as f64 / 16000.0);
    for (name, turns) in [("a", &turns_a), ("b", &turns_b)] {
        for (start, end, who) in turns.iter() {
            println!("turn {name} ({start:.2}, {end:.2}, \"{who}\"),");
        }
    }

    for model in &args[1..] {
        let name = Path::new(model).file_name().unwrap().to_string_lossy();
        println!("\n=== {name}");
        let engine = SpeakerEngine::load_with(Path::new(model), 0.5)?;
        let mut clean: BTreeMap<String, Vec<Vec<f32>>> = BTreeMap::new();
        for clip in &first {
            clean.entry(clip.speaker.clone()).or_default().push(engine.embedding(&clip.samples).unwrap());
        }
        let mut call: BTreeMap<String, Vec<Vec<f32>>> = BTreeMap::new();
        for clip in &call_clips {
            call.entry(clip.speaker.clone()).or_default().push(engine.embedding(&clip.samples).unwrap());
        }
        // Turn level: same person vs different people (within the clean session).
        let mut same = Vec::new();
        let mut different = Vec::new();
        let all: Vec<(&String, &Vec<f32>)> = clean.iter().flat_map(|(s, vs)| vs.iter().map(move |v| (s, v))).collect();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                let similarity = cosine(all[i].1, all[j].1);
                if all[i].0 == all[j].0 { same.push(similarity) } else { different.push(similarity) }
            }
        }
        let range = |v: &[f32]| format!("{:.2}…{:.2} (mean {:.2})", v.iter().cloned().fold(1.0, f32::min), v.iter().cloned().fold(-1.0, f32::max), v.iter().sum::<f32>() / v.len() as f32);
        println!("turns, same person:      {}", range(&same));
        println!("turns, different people: {}", range(&different));
        let speakers: Vec<String> = clean.keys().cloned().collect();
        for a in &speakers {
            let row: Vec<String> = speakers
                .iter()
                .map(|b| format!("{b}: {:.2}", cosine(&mean(&clean[a]), &mean(&call[b]))))
                .collect();
            println!("profile {a} (clean) vs call -> {}", row.join("  "));
        }
        if std::env::var("VERBOSE").is_ok() {
            let engine = SpeakerEngine::load_with(Path::new(model), 0.5)?;
            let d = engine.diarize(&dialogue_b, None)?;
            for (start, end, who) in &turns_b {
                println!("truth {start:6.2}–{end:6.2} {who}");
            }
            for turn in &d.turns {
                println!("found {:6.2}–{:6.2} {}", turn.start, turn.end, turn.speaker);
            }
        }
        for threshold in [0.3f32, 0.4, 0.5, 0.6, 0.7] {
            let engine = SpeakerEngine::load_with(Path::new(model), threshold)?;
            let (count_a, accuracy_a) = turn_accuracy(&engine.diarize(&dialogue_a, None)?, &turns_a);
            let (count_b, accuracy_b) = turn_accuracy(&engine.diarize(&dialogue_b, None)?, &turns_b);
            println!("threshold {threshold:.1}: clean {count_a} speakers ({:.0} %), call {count_b} speakers ({:.0} %)", accuracy_a * 100.0, accuracy_b * 100.0);
        }
    }
    Ok(())
}
