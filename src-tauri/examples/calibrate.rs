//! Prints how many speakers diarization finds per clustering threshold and
//! how similar the voices are (development tool).
//!
//! cargo run --release --example calibrate -- file.m4a [file2.m4a …]

use scripta_lib::audio::{mf, MODEL_RATE};
use scripta_lib::engine::SpeakerEngine;
use scripta_lib::voices::cosine;

fn main() -> anyhow::Result<()> {
    let model = std::env::var("SCRIPTA_EMBEDDING").ok();
    let engine = match &model {
        Some(path) => SpeakerEngine::load_with(std::path::Path::new(path), 0.5)?,
        None => SpeakerEngine::load()?,
    };
    for path in std::env::args().skip(1) {
        let samples = mf::decode_to_vec(std::path::Path::new(&path), MODEL_RATE, |_| {})?;
        println!("== {path} ({:.1} s)", samples.len() as f64 / MODEL_RATE as f64);
        for threshold in [0.2f32, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2] {
            let started = std::time::Instant::now();
            let d = engine.diarize_with_threshold(&samples, threshold)?;
            if threshold > 0.75 {
                break;
            }
            let speakers = d.speakers();
            let mut similarities = Vec::new();
            for (i, a) in speakers.iter().enumerate() {
                for b in &speakers[i + 1..] {
                    if let (Some(x), Some(y)) = (d.embeddings.get(a), d.embeddings.get(b)) {
                        similarities.push(format!("{a}-{b}: {:.2}", cosine(x, y)));
                    }
                }
            }
            println!(
                "threshold {threshold:.1}: {} speakers, {} turns, {:.1} s  {}",
                speakers.len(),
                d.turns.len(),
                started.elapsed().as_secs_f64(),
                similarities.join(", ")
            );
        }
        let two = engine.diarize(&samples, Some(2))?;
        if let (Some(a), Some(b)) = (two.embeddings.get(&1), two.embeddings.get(&2)) {
            println!("forced 2: similarity {:.2}", cosine(a, b));
        }
    }
    Ok(())
}
