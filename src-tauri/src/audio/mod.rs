//! Audio: capture, files, levelling, mixing, decoding and encoding.

pub mod capture;
pub mod devices;
pub mod leveler;
pub mod mf;
pub mod mixer;
pub mod resample;
pub mod wav;

/// Sample rate of captured tracks (mono).
pub const CAPTURE_RATE: u32 = 48_000;
/// Sample rate the speech models expect (mono).
pub const MODEL_RATE: u32 = 16_000;

/// Quietest level that still shows on a meter (dBFS).
const METER_FLOOR_DECIBELS: f32 = -50.0;

/// Converts an RMS amplitude to a 0...1 meter level.
pub fn normalized_level(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let decibels = 20.0 * rms.log10();
    ((decibels - METER_FLOOR_DECIBELS) / -METER_FLOOR_DECIBELS).clamp(0.0, 1.0)
}

/// RMS of consecutive blocks, fed buffer by buffer.
pub struct BlockLevels {
    block_frames: usize,
    sum_of_squares: f64,
    frame_count: usize,
}

impl BlockLevels {
    pub fn new(block_frames: usize) -> Self {
        Self { block_frames: block_frames.max(1), sum_of_squares: 0.0, frame_count: 0 }
    }

    /// Adds samples and returns the RMS of every block completed by them.
    pub fn add(&mut self, samples: &[f32], mut completed: impl FnMut(f32)) {
        for sample in samples {
            self.sum_of_squares += (*sample as f64) * (*sample as f64);
            self.frame_count += 1;
            if self.frame_count == self.block_frames {
                completed((self.sum_of_squares / self.block_frames as f64).sqrt() as f32);
                self.sum_of_squares = 0.0;
                self.frame_count = 0;
            }
        }
    }
}

/// Mono from interleaved samples.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved.chunks_exact(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_levels() {
        assert_eq!(normalized_level(0.0), 0.0);
        assert_eq!(normalized_level(1.0), 1.0);
        assert!((normalized_level(0.01) - 0.2).abs() < 0.001); // -40 dBFS
    }

    #[test]
    fn block_levels() {
        let mut levels = BlockLevels::new(4);
        let mut found = Vec::new();
        levels.add(&[0.5, -0.5, 0.5], |rms| found.push(rms));
        levels.add(&[-0.5, 0.0, 0.0, 0.0, 0.0], |rms| found.push(rms));
        assert_eq!(found, [0.5, 0.0]);
    }

    #[test]
    fn downmixing() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), [0.5, 0.5]);
    }
}
