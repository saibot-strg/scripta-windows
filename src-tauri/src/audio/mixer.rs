//! Mixes the separate meeting tracks into one file for playback.

use std::path::Path;

use anyhow::{anyhow, Result};

use super::wav::MonoWriter;
use super::CAPTURE_RATE;

/// Sums mono 48 kHz WAV files into `output` and returns its duration.
/// Missing or unreadable inputs are skipped; shorter inputs end in silence.
/// Processes one second at a time.
pub fn mix(inputs: &[&Path], output: &Path) -> Result<f64> {
    let mut readers: Vec<hound::WavReader<std::io::BufReader<std::fs::File>>> = inputs
        .iter()
        .filter_map(|path| hound::WavReader::open(path).ok())
        .filter(|r| r.spec().sample_rate == CAPTURE_RATE && r.spec().channels == 1 && r.spec().bits_per_sample == 16)
        .collect();
    if readers.is_empty() {
        return Err(anyhow!("no tracks to mix"));
    }
    let temporary = output.with_file_name(format!("mixing-{}.wav", uuid::Uuid::new_v4()));
    let result = (|| -> Result<f64> {
        let mut writer = MonoWriter::create(&temporary)?;
        let chunk = CAPTURE_RATE as usize;
        let mut mixed = vec![0f32; chunk];
        let mut iterators: Vec<_> = readers.iter_mut().map(|r| r.samples::<i16>()).collect();
        loop {
            mixed.iter_mut().for_each(|x| *x = 0.0);
            let mut frames = 0;
            for samples in iterators.iter_mut() {
                for (index, slot) in mixed.iter_mut().enumerate() {
                    match samples.next() {
                        Some(sample) => {
                            *slot += sample? as f32 / 32_768.0;
                            frames = frames.max(index + 1);
                        }
                        None => break,
                    }
                }
            }
            if frames == 0 {
                break;
            }
            // Keep the sum in range; overlapping speech rarely clips, but never wraps.
            for x in mixed[..frames].iter_mut() {
                *x = x.clamp(-1.0, 1.0);
            }
            writer.write(&mixed[..frames])?;
        }
        let duration = writer.finish()?;
        std::fs::rename(&temporary, output)?;
        Ok(duration)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::{read_chunks, write_mono};

    #[test]
    fn tracks_are_summed_and_shorter_ones_end_in_silence() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("microphone.wav");
        let b = dir.path().join("system.wav");
        let out = dir.path().join("mix.wav");
        write_mono(&a, &vec![0.25; 72_000], CAPTURE_RATE).unwrap();
        write_mono(&b, &vec![0.5; 24_000], CAPTURE_RATE).unwrap();
        let duration = mix(&[&a, &b, &dir.path().join("missing.wav")], &out).unwrap();
        assert_eq!(duration, 1.5);
        let mut samples = Vec::new();
        read_chunks(&out, |c| {
            samples.extend_from_slice(c);
            Ok(())
        })
        .unwrap();
        assert!((samples[0] - 0.75).abs() < 0.001);
        assert!((samples[50_000] - 0.25).abs() < 0.001);
    }
}
