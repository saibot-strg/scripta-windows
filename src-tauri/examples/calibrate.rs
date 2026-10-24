//! Calibrates speaker separation on a real recording (development tool): how
//! many speakers the clustering finds per threshold, how many are left after
//! consolidation (as in the app and with other limits), and how alike forced
//! speakers sound.
//!
//! cargo run --release --example calibrate -- file.m4a [start minute] [minutes]
//! SCRIPTA_THRESHOLDS=0.5,0.6 picks the clustering thresholds, SCRIPTA_FORCED=3,4
//! the fixed speaker counts to try ("" for none), SCRIPTA_GPU=1 (or a card's
//! name) runs the models on the graphics card, SCRIPTA_DUMP=dir writes the raw
//! clustering per threshold as JSON (voice data: keep it out of the repo),
//! SCRIPTA_WINDOW_SHIFT=0.25 another segmentation window shift for long audio.

use std::time::Instant;

use scripta_lib::audio::{mf, MODEL_RATE};
use scripta_lib::engine::{SpeakerEngine, CLUSTER_THRESHOLD, LONG_WINDOW_SHIFT};
use scripta_lib::gpu::Device;
use scripta_lib::models::ModelKind;
use scripta_lib::transcript::{consolidate, consolidate_with, Diarization};
use scripta_lib::voices::cosine;

/// Share of the speech per speaker, largest first.
fn shares(d: &Diarization) -> String {
    let mut speech: Vec<(i64, f64)> = d
        .speakers()
        .into_iter()
        .map(|s| (s, d.turns.iter().filter(|t| t.speaker == s).map(|t| t.end - t.start).sum()))
        .collect();
    let total: f64 = speech.iter().map(|(_, s)| s).sum::<f64>().max(1e-6);
    speech.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut parts: Vec<String> = speech.iter().take(6).map(|(_, s)| format!("{:.0}%", 100.0 * s / total)).collect();
    if speech.len() > 6 {
        parts.push("…".into());
    }
    parts.join(" ")
}

/// Only the `count` speakers with the most speech.
fn largest(d: &Diarization, count: usize) -> Diarization {
    let mut speech: Vec<(i64, f64)> = d
        .speakers()
        .into_iter()
        .map(|s| (s, d.turns.iter().filter(|t| t.speaker == s).map(|t| t.end - t.start).sum()))
        .collect();
    speech.sort_by(|a, b| b.1.total_cmp(&a.1));
    let keep: Vec<i64> = speech.iter().take(count).map(|(s, _)| *s).collect();
    Diarization {
        turns: d.turns.iter().filter(|t| keep.contains(&t.speaker)).copied().collect(),
        embeddings: d.embeddings.iter().filter(|(s, _)| keep.contains(s)).map(|(s, e)| (*s, e.clone())).collect(),
    }
}

fn similarities(d: &Diarization) -> String {
    let speakers = d.speakers();
    let mut values = Vec::new();
    for (i, a) in speakers.iter().enumerate() {
        for b in &speakers[i + 1..] {
            if let (Some(x), Some(y)) = (d.embeddings.get(a), d.embeddings.get(b)) {
                values.push(format!("{a}-{b} {:.2}", cosine(x, y)));
            }
        }
    }
    values.join(", ")
}

fn main() -> anyhow::Result<()> {
    // The engine's own log lines (timings) go to the console.
    tracing_subscriber::fmt().with_target(false).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start: f64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0.0);
    let minutes: f64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(f64::INFINITY);
    let thresholds: Vec<f32> = std::env::var("SCRIPTA_THRESHOLDS")
        .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
        .unwrap_or_else(|_| vec![0.5, 0.6, 0.7]);

    let all = mf::decode_to_vec(std::path::Path::new(&args[0]), MODEL_RATE, |_| {})?;
    let rate = MODEL_RATE as f64;
    let from = ((start * 60.0 * rate) as usize).min(all.len());
    let to = if minutes.is_finite() { ((start + minutes) * 60.0 * rate) as usize } else { all.len() };
    let samples = &all[from..to.min(all.len())];
    println!("{} from {start} min: {:.1} min", args[0], samples.len() as f64 / rate / 60.0);

    let device = Device::from_env();
    let cards: Vec<String> = scripta_lib::gpu::cards().into_iter().map(|c| format!("{} (LUID {:x}:{:x})", c.name, c.luid >> 32, c.luid & 0xffff_ffff)).collect();
    println!("{:?} of {cards:?}", device.card().map(|c| c.name));
    // SCRIPTA_WINDOW_SHIFT=0.25 tries another segmentation window shift.
    let shift = std::env::var("SCRIPTA_WINDOW_SHIFT").ok().and_then(|v| v.parse().ok()).unwrap_or(LONG_WINDOW_SHIFT);
    let engine = SpeakerEngine::load_with(&ModelKind::Speakers.file("titanet-small.onnx"), CLUSTER_THRESHOLD, shift, device)?;
    for threshold in thresholds {
        let started = Instant::now();
        let raw = engine.diarize_raw(samples, threshold)?;
        if let Ok(dir) = std::env::var("SCRIPTA_DUMP") {
            let turns: Vec<_> = raw.turns.iter().map(|t| serde_json::json!([t.speaker, t.start, t.end])).collect();
            let embeddings: std::collections::BTreeMap<String, &Vec<f32>> = raw.embeddings.iter().map(|(k, v)| (k.to_string(), v)).collect();
            let path = std::path::Path::new(&dir).join(format!("raw-{threshold:.2}.json"));
            std::fs::write(&path, serde_json::to_vec(&serde_json::json!({ "turns": turns, "embeddings": embeddings }))?)?;
        }
        println!("\nthreshold {threshold:.2}: {} speakers, {} turns, {:.0} s", raw.speakers().len(), raw.turns.len(), started.elapsed().as_secs_f64());
        for minimum_speech in [15.0, 30.0, 60.0] {
            let cells: Vec<String> = [0.4f32, 0.45, 0.5, 0.55, 0.6, 0.7, 0.8]
                .iter()
                .map(|same| {
                    let d = consolidate_with(raw.clone(), minimum_speech, *same);
                    format!("{same:.2}: {} ({})", d.speakers().len(), shares(&d))
                })
                .collect();
            println!("  fragments < {minimum_speech:>2.0} s: {}", cells.join("  "));
        }
        let d = consolidate(raw);
        println!("  as in the app: {} speakers ({})  {}", d.speakers().len(), shares(&d), similarities(&largest(&d, 6)));
    }
    let forced: Vec<usize> = std::env::var("SCRIPTA_FORCED")
        .map(|v| v.split(',').filter_map(|c| c.trim().parse().ok()).collect())
        .unwrap_or_else(|_| vec![2, 3, 4, 5]);
    for count in forced {
        let started = Instant::now();
        let d = engine.diarize(samples, Some(count))?;
        println!("\nforced {count} ({:.0} s): {}  {}", started.elapsed().as_secs_f64(), shares(&d), similarities(&d));
    }
    Ok(())
}
