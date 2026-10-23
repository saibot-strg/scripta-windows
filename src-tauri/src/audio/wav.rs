//! Captured tracks on disk: 16-bit mono WAV at 48 kHz.
//!
//! The header is rewritten every second, so after a crash the file is
//! readable up to the last second at least; `repair` fixes the sizes from
//! the file length on the next launch.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::{normalized_level, BlockLevels, CAPTURE_RATE};

pub fn spec() -> hound::WavSpec {
    hound::WavSpec { channels: 1, sample_rate: CAPTURE_RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

/// Frames per meter level (50 ms).
pub const LEVEL_BLOCK_FRAMES: usize = (CAPTURE_RATE / 20) as usize;
/// Gaps shorter than this are clock jitter, not silence.
const GAP_TOLERANCE_FRAMES: u64 = (CAPTURE_RATE / 10) as u64;
/// 100-ns units per second (WASAPI and QPC timestamps).
pub const UNITS_PER_SECOND: u64 = 10_000_000;

/// What a sink reports after writing.
pub struct SinkProgress {
    /// Meter levels (0...1), one per 50 ms.
    pub levels: Vec<f32>,
    /// Total written duration in seconds.
    pub duration: f64,
}

/// Receives mono 48 kHz audio on the capture thread and appends it to the
/// WAV file immediately.
///
/// With a timeline (all recordings), every buffer is placed by its capture
/// time: gaps are filled with silence. System audio only delivers buffers
/// while some app plays sound, so without this the "Andere" track would lose
/// all silent stretches and drift ahead of "Ich".
pub struct TrackSink {
    path: PathBuf,
    writer: Option<hound::WavWriter<BufWriter<File>>>,
    frames_written: u64,
    /// Second zero of the recording (100-ns units); all tracks share it.
    origin: Option<u64>,
    paused_at: Option<u64>,
    meter: BlockLevels,
    last_flush: Instant,
    failed: bool,
}

impl TrackSink {
    pub fn create(path: &Path) -> Result<Self> {
        let writer = hound::WavWriter::create(path, spec()).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: Some(writer),
            frames_written: 0,
            origin: None,
            paused_at: None,
            meter: BlockLevels::new(LEVEL_BLOCK_FRAMES),
            last_flush: Instant::now(),
            failed: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn start_timeline(&mut self, origin: u64) {
        self.origin = Some(origin);
    }

    pub fn pause_timeline(&mut self, at: u64) {
        self.paused_at = Some(at);
    }

    /// Shifts the origin by the pause, so paused time isn't filled with silence.
    pub fn resume_timeline(&mut self, at: u64) {
        if let (Some(paused), Some(origin)) = (self.paused_at.take(), self.origin) {
            self.origin = Some(origin + at.saturating_sub(paused.min(at)));
        }
    }

    pub fn duration(&self) -> f64 {
        self.frames_written as f64 / CAPTURE_RATE as f64
    }

    /// Places and writes one buffer captured at `timestamp` (100-ns units,
    /// time of its first frame). Returns meter levels, or an error message
    /// once writing failed (the file keeps everything up to that point).
    pub fn append(&mut self, samples: &[f32], timestamp: u64) -> Result<Option<SinkProgress>, String> {
        if self.failed {
            return Ok(None);
        }
        let Some(origin) = self.origin else { return Ok(None) };
        if self.paused_at.is_some() || timestamp < origin {
            return Ok(None);
        }
        let result = (|| -> Result<Option<SinkProgress>> {
            let Some(writer) = self.writer.as_mut() else { return Ok(None) };
            let expected = (timestamp - origin) as u128 * CAPTURE_RATE as u128 / UNITS_PER_SECOND as u128;
            let expected = expected as u64;
            if expected > self.frames_written + GAP_TOLERANCE_FRAMES {
                let gap = expected - self.frames_written;
                for _ in 0..gap {
                    writer.write_sample(0i16)?;
                }
                self.frames_written += gap;
            }
            for sample in samples {
                writer.write_sample(to_i16(*sample))?;
            }
            self.frames_written += samples.len() as u64;
            if self.last_flush.elapsed() >= Duration::from_secs(1) {
                writer.flush()?;
                self.last_flush = Instant::now();
            }
            let mut levels = Vec::new();
            self.meter.add(samples, |rms| levels.push(normalized_level(rms)));
            Ok(Some(SinkProgress { levels, duration: self.duration() }))
        })();
        result.map_err(|error| {
            self.failed = true;
            error.to_string()
        })
    }

    /// Closes the file and returns the recorded duration in seconds.
    pub fn finish(&mut self) -> f64 {
        if let Some(writer) = self.writer.take() {
            if let Err(error) = writer.finalize() {
                tracing::error!(%error, "closing track failed");
            }
        }
        self.duration()
    }
}

/// Reads a WAV file as mono f32 in chunks of about one second.
pub fn read_chunks(path: &Path, mut chunk: impl FnMut(&[f32]) -> Result<()>) -> Result<u32> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let per_chunk = spec.sample_rate as usize * channels;
    let mut interleaved: Vec<f32> = Vec::with_capacity(per_chunk);
    let scale = match spec.sample_format {
        hound::SampleFormat::Int => 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32,
        hound::SampleFormat::Float => 1.0,
    };
    macro_rules! drain {
        () => {{
            let mono = super::downmix(&interleaved, channels);
            chunk(&mono)?;
            interleaved.clear();
        }};
    }
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                interleaved.push(sample?);
                if interleaved.len() == per_chunk {
                    drain!();
                }
            }
        }
        hound::SampleFormat::Int => {
            for sample in reader.samples::<i32>() {
                interleaved.push(sample? as f32 * scale);
                if interleaved.len() == per_chunk {
                    drain!();
                }
            }
        }
    }
    if !interleaved.is_empty() {
        drain!();
    }
    Ok(spec.sample_rate)
}

/// Writes mono f32 samples as a 16-bit WAV at `rate`.
pub fn write_mono(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    let mut writer = hound::WavWriter::create(path, hound::WavSpec { sample_rate: rate, ..spec() })?;
    for sample in samples {
        writer.write_sample(to_i16(*sample))?;
    }
    writer.finalize()?;
    Ok(())
}

/// Opens a WAV writer for streaming 48 kHz mono output.
pub struct MonoWriter {
    writer: hound::WavWriter<BufWriter<File>>,
    frames: u64,
}

impl MonoWriter {
    pub fn create(path: &Path) -> Result<Self> {
        Ok(Self { writer: hound::WavWriter::create(path, spec())?, frames: 0 })
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        for sample in samples {
            self.writer.write_sample(to_i16(*sample))?;
        }
        self.frames += samples.len() as u64;
        Ok(())
    }

    /// Closes the file and returns its duration in seconds.
    pub fn finish(self) -> Result<f64> {
        self.writer.finalize()?;
        Ok(self.frames as f64 / CAPTURE_RATE as f64)
    }
}

/// Duration of a WAV file in seconds (after `repair`).
pub fn duration(path: &Path) -> Option<f64> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    Some(reader.duration() as f64 / spec.sample_rate.max(1) as f64)
}

/// Fixes the RIFF and data sizes of a WAV file that was not closed (crash
/// while recording). Returns true if the file was changed.
pub fn repair(path: &Path) -> Result<bool> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let length = file.metadata()?.len();
    if length < 44 {
        return Ok(false);
    }
    let mut header = [0u8; 12];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Ok(false);
    }
    let mut changed = false;
    let riff_size = (length - 8).min(u32::MAX as u64) as u32;
    if u32::from_le_bytes([header[4], header[5], header[6], header[7]]) != riff_size {
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&riff_size.to_le_bytes())?;
        changed = true;
    }
    // Walk the chunks to the data chunk.
    let mut position = 12u64;
    while position + 8 <= length {
        file.seek(SeekFrom::Start(position))?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as u64;
        if &chunk[0..4] == b"data" {
            // Whole frames of 16-bit mono.
            let actual = ((length - position - 8) / 2 * 2).min(u32::MAX as u64) as u32;
            if size != actual as u64 {
                file.seek(SeekFrom::Start(position + 4))?;
                file.write_all(&actual.to_le_bytes())?;
                changed = true;
            }
            break;
        }
        position += 8 + size + (size & 1);
    }
    file.flush()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_fills_gaps_with_silence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        let mut sink = TrackSink::create(&path).unwrap();
        let origin = 1_000_000_000;
        // Before the timeline starts nothing is written.
        assert!(sink.append(&[0.5; 480], origin).unwrap().is_none());
        sink.start_timeline(origin);
        sink.append(&[0.5; 4_800], origin).unwrap();
        // The next buffer arrives 2 s after the origin: 1.9 s of silence first.
        sink.append(&[0.5; 4_800], origin + 2 * UNITS_PER_SECOND).unwrap();
        assert_eq!(sink.finish(), 2.1);
        assert_eq!(duration(&path), Some(2.1));
    }

    #[test]
    fn pauses_are_not_filled() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = TrackSink::create(&dir.path().join("mic.wav")).unwrap();
        let origin = 5 * UNITS_PER_SECOND;
        sink.start_timeline(origin);
        sink.append(&[0.1; 48_000], origin).unwrap();
        sink.pause_timeline(origin + UNITS_PER_SECOND);
        assert!(sink.append(&[0.1; 4_800], origin + 2 * UNITS_PER_SECOND).unwrap().is_none());
        sink.resume_timeline(origin + 11 * UNITS_PER_SECOND);
        sink.append(&[0.1; 48_000], origin + 11 * UNITS_PER_SECOND).unwrap();
        assert_eq!(sink.finish(), 2.0);
    }

    #[test]
    fn crashed_files_are_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recording.wav");
        write_mono(&path, &[0.1; 48_000], CAPTURE_RATE).unwrap();
        // Simulate a crash: another second is on disk, the header doesn't know.
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&vec![0u8; 96_000]).unwrap();
        drop(file);
        assert_eq!(duration(&path), Some(1.0));
        assert!(repair(&path).unwrap());
        assert_eq!(duration(&path), Some(2.0));
        assert!(!repair(&path).unwrap());
    }
}
