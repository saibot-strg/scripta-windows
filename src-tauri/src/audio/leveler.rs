//! Raises quiet microphone recordings to a normal speech level.
//! A port of Scripta's `MicrophoneLeveler`.
//!
//! Some headsets deliver speech 15–20 dB below other sources even at full
//! input volume; meeting apps hide this with their own gain control. The
//! leveler measures how loud the speech is, applies one fixed gain to the
//! whole recording (so its dynamics stay as they are) and limits the few
//! peaks that would then clip. Recordings that are loud enough stay untouched.

use std::path::Path;

use anyhow::Result;

use super::wav::{read_chunks, MonoWriter};
use super::BlockLevels;

/// Speech level to reach: the 90th percentile of the 50 ms blocks that carry
/// speech, in dBFS RMS. Meeting participants in system audio land around here.
pub const TARGET_DECIBELS: f32 = -18.0;
/// Upper bound, so a recording without real speech doesn't turn into loud noise.
pub const MAXIMUM_GAIN_DECIBELS: f32 = 24.0;
/// Smaller corrections aren't worth rewriting the file.
pub const MINIMUM_GAIN_DECIBELS: f32 = 1.5;
/// Blocks this far above the noise floor count as speech.
const SPEECH_ABOVE_FLOOR_DECIBELS: f32 = 12.0;
/// At least this much speech (50 ms blocks, i.e. 0.5 s) before the level is trusted.
const MINIMUM_SPEECH_BLOCKS: usize = 10;
/// Blocks below this are digital silence (e.g. gaps on the timeline).
const SILENCE_DECIBELS: f32 = -70.0;

/// Gain in dB that brings the speech in `block_decibels` (RMS of 50 ms
/// blocks) to the target, or None when no correction is needed or possible.
pub fn gain_decibels(block_decibels: &[f32]) -> Option<f32> {
    let mut audible: Vec<f32> = block_decibels.iter().copied().filter(|d| *d > SILENCE_DECIBELS).collect();
    if audible.len() < MINIMUM_SPEECH_BLOCKS {
        return None;
    }
    audible.sort_by(f32::total_cmp);
    let noise_floor = audible[audible.len() / 10];
    // Still sorted, so the percentile is a plain index.
    let speech: Vec<f32> = audible.into_iter().filter(|d| *d >= noise_floor + SPEECH_ABOVE_FLOOR_DECIBELS).collect();
    if speech.len() < MINIMUM_SPEECH_BLOCKS {
        return None;
    }
    let speech_level = speech[speech.len() * 9 / 10];
    let gain = (TARGET_DECIBELS - speech_level).min(MAXIMUM_GAIN_DECIBELS);
    (gain >= MINIMUM_GAIN_DECIBELS).then_some(gain)
}

fn decibels(rms: f32) -> f32 {
    if rms > 0.0 {
        20.0 * rms.log10()
    } else {
        -120.0
    }
}

/// Levels mono samples in place (dictation). Returns the applied gain in dB.
pub fn level(samples: &mut [f32], sample_rate: u32) -> Option<f32> {
    let mut levels = BlockLevels::new((sample_rate / 20) as usize);
    let mut blocks = Vec::new();
    levels.add(samples, |rms| blocks.push(decibels(rms)));
    let gain = gain_decibels(&blocks)?;
    PeakLimiter::new(gain, sample_rate).process(samples);
    Some(gain)
}

/// Levels a captured WAV file in place. The levelled audio is written next
/// to it and swapped in, so the original survives a crash. Returns the
/// applied gain in dB, or None if the file stayed as it was.
pub fn level_file(path: &Path) -> Result<Option<f32>> {
    if !path.exists() {
        return Ok(None);
    }
    // Pass 1: how loud is the speech?
    let mut blocks = Vec::new();
    let mut levels = BlockLevels::new((super::CAPTURE_RATE / 20) as usize);
    let rate = read_chunks(path, |chunk| {
        levels.add(chunk, |rms| blocks.push(decibels(rms)));
        Ok(())
    })?;
    if rate != super::CAPTURE_RATE {
        return Ok(None);
    }
    let Some(gain) = gain_decibels(&blocks) else { return Ok(None) };

    // Pass 2: write the levelled copy.
    let temporary = path.with_file_name(format!("levelling-{}.wav", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut writer = MonoWriter::create(&temporary)?;
        let mut limiter = PeakLimiter::new(gain, rate);
        let mut buffer = Vec::new();
        read_chunks(path, |chunk| {
            buffer.clear();
            buffer.extend_from_slice(chunk);
            limiter.process(&mut buffer);
            writer.write(&buffer)
        })?;
        writer.finish()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    tracing::info!(gain, file = %path.file_name().unwrap_or_default().to_string_lossy(), "levelled microphone");
    Ok(Some(gain))
}

/// Applies a fixed gain and keeps every sample below `CEILING`: instant
/// attack, about 100 ms release. Keeps its state across buffers.
pub struct PeakLimiter {
    gain: f32,
    release: f32,
    envelope: f32,
}

impl PeakLimiter {
    /// -1 dBFS, headroom for the AAC encoder.
    pub const CEILING: f32 = 0.891;

    pub fn new(gain_decibels: f32, sample_rate: u32) -> Self {
        Self {
            gain: 10f32.powf(gain_decibels / 20.0),
            release: (-1.0 / (0.1 * sample_rate as f64)).exp() as f32,
            envelope: 0.0,
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let mut envelope = self.envelope;
        for sample in samples.iter_mut() {
            *sample *= self.gain;
            envelope = sample.abs().max(envelope * self.release);
            // The envelope is never below the current sample, so this lands at or under the ceiling.
            if envelope > Self::CEILING {
                *sample *= Self::CEILING / envelope;
            }
        }
        self.envelope = envelope;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::write_mono;
    use crate::audio::CAPTURE_RATE;

    /// Speech stand-in: 400 ms tone bursts at `speech_db` RMS, separated by
    /// 300 ms of a quiet hum at -65 dB.
    fn speech_like(seconds: f64, speech_db: f32) -> Vec<f32> {
        let rate = CAPTURE_RATE as f64;
        let speech_amplitude = 10f32.powf(speech_db / 20.0) * 2f32.sqrt();
        let noise_amplitude = 10f32.powf(-65.0 / 20.0) * 2f32.sqrt();
        (0..(rate * seconds) as usize)
            .map(|i| {
                let time = i as f64 / rate;
                let is_speech = time % 0.7 < 0.4;
                let amplitude = if is_speech { speech_amplitude } else { noise_amplitude };
                amplitude * (2.0 * std::f64::consts::PI * if is_speech { 440.0 } else { 1_000.0 } * time).sin() as f32
            })
            .collect()
    }

    fn read(path: &Path) -> Vec<f32> {
        let mut all = Vec::new();
        read_chunks(path, |chunk| {
            all.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
        all
    }

    fn burst_decibels(samples: &[f32]) -> f32 {
        let rate = CAPTURE_RATE as f64;
        let burst = &samples[(rate * 0.05) as usize..(rate * 0.35) as usize];
        decibels((burst.iter().map(|x| x * x).sum::<f32>() / burst.len() as f32).sqrt())
    }

    #[test]
    fn gain_decision() {
        let noise = vec![-65.0f32; 60];
        let with = |speech: f32, count: usize| [noise.clone(), vec![speech; count]].concat();
        assert_eq!(gain_decibels(&with(-32.0, 40)), Some(14.0));
        assert_eq!(gain_decibels(&with(-17.0, 40)), None);
        assert_eq!(gain_decibels(&noise), None);
        assert_eq!(gain_decibels(&[-120.0; 200]), None);
        assert_eq!(gain_decibels(&with(-32.0, 5)), None);
    }

    #[test]
    fn gain_is_capped() {
        let blocks = [vec![-69.0f32; 60], vec![-50.0; 40]].concat();
        assert_eq!(gain_decibels(&blocks), Some(MAXIMUM_GAIN_DECIBELS));
    }

    #[test]
    fn quiet_files_are_levelled_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.wav");
        write_mono(&path, &speech_like(5.0, -32.0), CAPTURE_RATE).unwrap();
        let gain = level_file(&path).unwrap().unwrap();
        assert!((gain - 14.0).abs() < 0.5, "gain {gain}");
        let samples = read(&path);
        assert_eq!(samples.len(), 5 * 48_000);
        let level = burst_decibels(&samples);
        assert!((level - TARGET_DECIBELS).abs() < 0.5, "level {level}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn peaks_are_limited() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.wav");
        let mut samples = speech_like(5.0, -40.0);
        samples[24_000] = 0.6; // a bump against the microphone, in a pause
        write_mono(&path, &samples, CAPTURE_RATE).unwrap();
        level_file(&path).unwrap();
        let levelled = read(&path);
        let peak = levelled.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak <= PeakLimiter::CEILING + 0.0001, "peak {peak}");
        assert!((burst_decibels(&levelled) - TARGET_DECIBELS).abs() < 0.5);
    }

    #[test]
    fn loud_files_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.wav");
        write_mono(&path, &speech_like(3.0, -16.0), CAPTURE_RATE).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(level_file(&path).unwrap(), None);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
