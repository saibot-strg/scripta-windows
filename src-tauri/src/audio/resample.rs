//! Streaming sample rate conversion (polyphase windowed sinc).
//!
//! Good enough for speech and playback: about 90 dB stopband, the band edge
//! at 95 % of the lower Nyquist frequency. Works on chunks of any size, so
//! hour-long files never have to be in memory at the source rate.

/// Zero crossings of the sinc on each side (at the lower of both rates).
const ZERO_CROSSINGS: f64 = 12.0;
/// More phases than this are quantized (unusual rate pairs).
const MAX_PHASES: u64 = 2048;

pub struct Resampler {
    passthrough: bool,
    up: u64,
    down: u64,
    half: usize,
    phases: u64,
    taps: Vec<f32>,
    /// Input samples; `buffer[0]` is input index `buffer_start` (negative
    /// at first: zeros before the stream).
    buffer: Vec<f32>,
    buffer_start: i64,
    next_output: u64,
    input_count: u64,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

/// Blackman window on -1...1.
fn blackman(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        return 0.0;
    }
    let t = (x + 1.0) / 2.0;
    0.42 - 0.5 * (2.0 * std::f64::consts::PI * t).cos() + 0.08 * (4.0 * std::f64::consts::PI * t).cos()
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let from = from.max(1) as u64;
        let to = to.max(1) as u64;
        if from == to {
            return Self {
                passthrough: true,
                up: 1,
                down: 1,
                half: 0,
                phases: 1,
                taps: Vec::new(),
                buffer: Vec::new(),
                buffer_start: 0,
                next_output: 0,
                input_count: 0,
            };
        }
        let divisor = gcd(from, to);
        let (up, down) = (to / divisor, from / divisor);
        let cutoff = (up as f64 / down as f64).min(1.0) * 0.95;
        let half = (ZERO_CROSSINGS / cutoff).ceil() as usize;
        let phases = up.min(MAX_PHASES);
        let width = 2 * half;
        let mut taps = vec![0f32; phases as usize * width];
        for phase in 0..phases as usize {
            let fraction = phase as f64 / phases as f64;
            let row = &mut taps[phase * width..(phase + 1) * width];
            let mut sum = 0.0;
            let mut values = vec![0.0f64; width];
            for (j, value) in values.iter_mut().enumerate() {
                // Offsets -half+1 ..= half around the output position.
                let t = (j as f64 - half as f64 + 1.0) - fraction;
                *value = cutoff * sinc(cutoff * t) * blackman(t / half as f64);
                sum += *value;
            }
            for (tap, value) in row.iter_mut().zip(values) {
                *tap = (value / sum) as f32;
            }
        }
        Self {
            passthrough: false,
            up,
            down,
            half,
            phases,
            taps,
            buffer: vec![0.0; half],
            buffer_start: -(half as i64),
            next_output: 0,
            input_count: 0,
        }
    }

    /// Converts the next chunk; output is appended to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.input_count += input.len() as u64;
        self.buffer.extend_from_slice(input);
        self.produce(out, u64::MAX);
    }

    /// Flushes the rest after the last chunk.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if self.passthrough {
            return;
        }
        let total = (self.input_count * self.up).div_ceil(self.down);
        self.buffer.extend(std::iter::repeat_n(0.0, 2 * self.half + 1));
        self.produce(out, total);
    }

    fn produce(&mut self, out: &mut Vec<f32>, limit: u64) {
        let width = 2 * self.half;
        let buffer_end = self.buffer_start + self.buffer.len() as i64;
        while self.next_output < limit {
            let position = self.next_output * self.down;
            let index = (position / self.up) as i64;
            if index + self.half as i64 >= buffer_end {
                break;
            }
            let remainder = position % self.up;
            let phase = if self.phases == self.up { remainder } else { remainder * self.phases / self.up };
            let row = &self.taps[phase as usize * width..(phase as usize + 1) * width];
            let base = (index - self.half as i64 + 1 - self.buffer_start) as usize;
            let window = &self.buffer[base..base + width];
            let value: f32 = window.iter().zip(row).map(|(x, h)| x * h).sum();
            out.push(value);
            self.next_output += 1;
        }
        // Drop what no later output needs.
        let needed_from = ((self.next_output * self.down) / self.up) as i64 - self.half as i64;
        if needed_from > self.buffer_start {
            let drop = ((needed_from - self.buffer_start) as usize).min(self.buffer.len());
            self.buffer.drain(..drop);
            self.buffer_start += drop as i64;
        }
    }
}

/// Converts a whole buffer at once.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let mut resampler = Resampler::new(from, to);
    let mut out = Vec::with_capacity(input.len() * to as usize / from.max(1) as usize + 16);
    resampler.process(input, &mut out);
    resampler.finish(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f64, rate: u32, seconds: f64) -> Vec<f32> {
        (0..(rate as f64 * seconds) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * frequency * i as f64 / rate as f64).sin() as f32 * 0.5)
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn lengths_match_the_ratio() {
        for (from, to) in [(48_000, 16_000), (44_100, 16_000), (16_000, 48_000), (22_050, 48_000), (48_000, 48_000)] {
            let input = sine(440.0, from, 1.0);
            let output = resample(&input, from, to);
            assert_eq!(output.len(), to as usize, "{from} -> {to}");
        }
    }

    #[test]
    fn passband_is_kept_and_stopband_removed() {
        // 1 kHz passes 48k -> 16k unchanged in level ...
        let kept = resample(&sine(1_000.0, 48_000, 1.0), 48_000, 16_000);
        assert!((rms(&kept[1000..15000]) - 0.3536).abs() < 0.01);
        // ... 12 kHz (above the new Nyquist) is gone.
        let removed = resample(&sine(12_000.0, 48_000, 1.0), 48_000, 16_000);
        assert!(rms(&removed[1000..15000]) < 0.001);
    }

    #[test]
    fn chunked_equals_whole() {
        let input = sine(300.0, 44_100, 0.5);
        let whole = resample(&input, 44_100, 16_000);
        let mut resampler = Resampler::new(44_100, 16_000);
        let mut chunked = Vec::new();
        for chunk in input.chunks(777) {
            resampler.process(chunk, &mut chunked);
        }
        resampler.finish(&mut chunked);
        assert_eq!(whole.len(), chunked.len());
        assert!(whole.iter().zip(&chunked).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn timing_is_preserved() {
        // An impulse at 0.5 s stays at 0.5 s.
        let mut input = vec![0f32; 48_000];
        input[24_000] = 1.0;
        let output = resample(&input, 48_000, 16_000);
        let peak = output.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((peak as i64 - 8_000).abs() <= 1, "{peak}");
    }
}
