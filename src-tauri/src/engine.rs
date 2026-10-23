//! Speech to text (Parakeet TDT 0.6B v3 through sherpa-onnx, CPU) and
//! speaker diarization (pyannote segmentation 3.0 + NVIDIA TitaNet small).
//!
//! Long audio is cut at pauses (Silero VAD) into chunks of up to ~25 s and
//! transcribed chunk by chunk, so memory stays flat and progress is real.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};
use sherpa_onnx::{
    FastClusteringConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineSpeakerDiarization,
    OfflineSpeakerDiarizationConfig, OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    OfflineTransducerModelConfig, SileroVadModelConfig, SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig,
    VadModelConfig, VoiceActivityDetector,
};

use crate::audio::MODEL_RATE;
use crate::model::WordTiming;
use crate::models::ModelKind;
use crate::transcript::{words_from_tokens, Diarization, SpeakerTurn, TokenTiming};

/// ONNX Runtime threads: half the cores, so the PC stays usable.
pub fn thread_count() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores / 2).clamp(2, 8) as i32
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("cancelled")]
    Cancelled,
}

fn path(kind: ModelKind, file: &str) -> Option<String> {
    Some(kind.file(file).to_string_lossy().into_owned())
}

// MARK: - Speech

pub struct SpeechEngine {
    recognizer: OfflineRecognizer,
}

/// Longest chunk handed to the recognizer.
const MAX_CHUNK_SECONDS: f64 = 25.0;
/// Pauses shorter than this inside a chunk are kept (context for the model).
const MERGE_GAP_SECONDS: f64 = 1.0;
/// Audio added before and after each chunk, so word edges aren't cut.
const PADDING_SECONDS: f64 = 0.3;
/// Silence appended to each chunk before decoding (0.5 s).
const TAIL_PADDING: usize = MODEL_RATE as usize / 2;

impl SpeechEngine {
    pub fn load() -> Result<Self> {
        let kind = ModelKind::Speech;
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: path(kind, "encoder.int8.onnx"),
            decoder: path(kind, "decoder.int8.onnx"),
            joiner: path(kind, "joiner.int8.onnx"),
        };
        config.model_config.tokens = path(kind, "tokens.txt");
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = thread_count();
        config.model_config.provider = Some("cpu".into());
        config.decoding_method = Some("greedy_search".into());
        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| anyhow!("the speech model could not be loaded"))?;
        Ok(Self { recognizer })
    }

    /// Words with timings in 16 kHz mono `samples`. `progress` gets 0...1.
    pub fn transcribe(&self, samples: &[f32], cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<Vec<WordTiming>> {
        let chunks = speech_chunks(samples)?;
        progress(0.05);
        let total = samples.len().max(1) as f64;
        let mut words = Vec::new();
        for (start, end) in chunks {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled.into());
            }
            words.extend(self.transcribe_chunk(&samples[start..end], start as f64 / MODEL_RATE as f64));
            progress(0.05 + 0.95 * end as f64 / total);
        }
        progress(1.0);
        Ok(words)
    }

    /// Short audio in memory (no chunking), e.g. a test or dictation.
    pub fn transcribe_short(&self, samples: &[f32]) -> Vec<WordTiming> {
        self.transcribe_chunk(samples, 0.0)
    }

    fn transcribe_chunk(&self, samples: &[f32], offset: f64) -> Vec<WordTiming> {
        // Parakeet needs at least ~0.3 s.
        if samples.len() < (MODEL_RATE as usize * 3 / 10) {
            return Vec::new();
        }
        // Transducers emit the last tokens only with some audio after them.
        let mut padded = Vec::with_capacity(samples.len() + TAIL_PADDING);
        padded.extend_from_slice(samples);
        padded.resize(samples.len() + TAIL_PADDING, 0.0);
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(MODEL_RATE as i32, &padded);
        self.recognizer.decode(&stream);
        let Some(result) = stream.get_result() else { return Vec::new() };
        let starts = result.timestamps.unwrap_or_default();
        let durations = result.durations.unwrap_or_default();
        let tokens: Vec<TokenTiming> = result
            .tokens
            .iter()
            .enumerate()
            .map(|(i, token)| {
                let start = starts.get(i).copied().unwrap_or(0.0) as f64;
                let end = match durations.get(i) {
                    Some(d) if *d > 0.0 => start + *d as f64,
                    _ => starts.get(i + 1).map(|s| *s as f64).unwrap_or(start + 0.08),
                };
                TokenTiming { token: token.clone(), start: offset + start, end: offset + end, confidence: None }
            })
            .collect();
        words_from_tokens(&tokens)
    }
}

/// Sample ranges with speech, merged into chunks of up to 25 s and padded.
fn speech_chunks(samples: &[f32]) -> Result<Vec<(usize, usize)>> {
    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: path(ModelKind::Speech, "silero_vad.onnx"),
            threshold: 0.4,
            min_silence_duration: 0.3,
            min_speech_duration: 0.15,
            window_size: 512,
            max_speech_duration: 20.0,
        },
        sample_rate: MODEL_RATE as i32,
        num_threads: 1,
        provider: Some("cpu".into()),
        ..Default::default()
    };
    let vad = VoiceActivityDetector::create(&config, 60.0).ok_or_else(|| anyhow!("the voice detector could not be loaded"))?;
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut collect = |vad: &VoiceActivityDetector| {
        while let Some(segment) = vad.front() {
            let start = segment.start().max(0) as usize;
            segments.push((start, start + segment.n().max(0) as usize));
            vad.pop();
        }
    };
    for window in samples.chunks(512) {
        vad.accept_waveform(window);
        collect(&vad);
    }
    vad.flush();
    collect(&vad);
    Ok(merge_segments(&segments, samples.len()))
}

/// Joins speech segments separated by short pauses into chunks of at most
/// `MAX_CHUNK_SECONDS` and pads them (without overlapping neighbors).
fn merge_segments(segments: &[(usize, usize)], length: usize) -> Vec<(usize, usize)> {
    let rate = MODEL_RATE as f64;
    let max_chunk = (MAX_CHUNK_SECONDS * rate) as usize;
    let merge_gap = (MERGE_GAP_SECONDS * rate) as usize;
    let mut chunks: Vec<(usize, usize)> = Vec::new();
    for &(start, end) in segments {
        let end = end.min(length);
        if start >= end {
            continue;
        }
        match chunks.last_mut() {
            Some(last) if start.saturating_sub(last.1) <= merge_gap && end - last.0 <= max_chunk => last.1 = end,
            _ => chunks.push((start, end)),
        }
    }
    let padding = (PADDING_SECONDS * rate) as usize;
    (0..chunks.len())
        .map(|index| {
            let (start, end) = chunks[index];
            // Padding stops halfway to the neighbors, so chunks never overlap.
            let lower = match index.checked_sub(1).map(|i| chunks[i].1) {
                Some(previous_end) => start.saturating_sub(padding).max((previous_end + start) / 2),
                None => start.saturating_sub(padding),
            };
            let upper = match chunks.get(index + 1) {
                Some(next) => (end + padding).min((end + next.0) / 2),
                None => (end + padding).min(length),
            };
            (lower, upper)
        })
        .collect()
}

// MARK: - Speakers

pub struct SpeakerEngine {
    diarizer: OfflineSpeakerDiarization,
    extractor: SpeakerEmbeddingExtractor,
    config: OfflineSpeakerDiarizationConfig,
}

/// Clustering threshold for an unknown number of speakers (larger means
/// fewer speakers). With TitaNet small, synthetic dialogues (German and
/// English, clean and call-like) separated correctly from 0.4 to 0.6.
pub const CLUSTER_THRESHOLD: f32 = 0.5;

impl SpeakerEngine {
    pub fn load() -> Result<Self> {
        Self::load_with(&ModelKind::Speakers.file("titanet-small.onnx"), CLUSTER_THRESHOLD)
    }

    /// With another embedding model and threshold (calibration).
    pub fn load_with(embedding_model: &std::path::Path, threshold: f32) -> Result<Self> {
        let kind = ModelKind::Speakers;
        let embedding = SpeakerEmbeddingExtractorConfig {
            model: Some(embedding_model.to_string_lossy().into_owned()),
            num_threads: thread_count(),
            debug: false,
            provider: Some("cpu".into()),
        };
        let config = OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                    model: path(kind, "segmentation.onnx"),
                    ..Default::default()
                },
                num_threads: thread_count(),
                ..Default::default()
            },
            embedding: embedding.clone(),
            clustering: FastClusteringConfig { num_clusters: -1, threshold, ..Default::default() },
            min_duration_on: 0.3,
            min_duration_off: 0.5,
        };
        let diarizer =
            OfflineSpeakerDiarization::create(&config).ok_or_else(|| anyhow!("the speaker model could not be loaded"))?;
        let extractor =
            SpeakerEmbeddingExtractor::create(&embedding).ok_or_else(|| anyhow!("the voice model could not be loaded"))?;
        Ok(Self { diarizer, extractor, config })
    }

    /// Who spoke when in 16 kHz mono `samples`, exactly `speaker_count`
    /// speakers if given, plus one voice fingerprint per speaker.
    pub fn diarize(&self, samples: &[f32], speaker_count: Option<usize>) -> Result<Diarization> {
        let mut config = self.config.clone();
        if let Some(count) = speaker_count {
            config.clustering.num_clusters = count as i32;
        }
        self.run(samples, &config)
    }

    /// Like `diarize` with another clustering threshold (calibration).
    pub fn diarize_with_threshold(&self, samples: &[f32], threshold: f32) -> Result<Diarization> {
        let mut config = self.config.clone();
        config.clustering.threshold = threshold;
        self.run(samples, &config)
    }

    fn run(&self, samples: &[f32], config: &OfflineSpeakerDiarizationConfig) -> Result<Diarization> {
        self.diarizer.set_config(config);
        let result = self.diarizer.process(samples).ok_or_else(|| anyhow!("speaker separation failed"))?;
        let mut numbers: HashMap<i32, i64> = HashMap::new();
        let mut turns = Vec::new();
        for segment in result.sort_by_start_time() {
            if segment.end <= segment.start {
                continue;
            }
            let next = numbers.len() as i64 + 1;
            let speaker = *numbers.entry(segment.speaker).or_insert(next);
            turns.push(SpeakerTurn { speaker, start: segment.start as f64, end: segment.end as f64 });
        }
        let mut embeddings = HashMap::new();
        for speaker in numbers.values() {
            if let Some(voice) = self.voice(samples, &turns, *speaker) {
                embeddings.insert(*speaker, voice);
            }
        }
        let diarization = Diarization { turns, embeddings };
        // A number the user asked for is kept as it is.
        if config.clustering.num_clusters > 0 {
            return Ok(diarization);
        }
        let found = diarization.speakers().len();
        let consolidated = crate::transcript::consolidate(diarization);
        if consolidated.speakers().len() != found {
            tracing::info!(found, kept = consolidated.speakers().len(), "speakers consolidated");
        }
        Ok(consolidated)
    }

    /// The mean embedding over the speaker's longest turns.
    fn voice(&self, samples: &[f32], turns: &[SpeakerTurn], speaker: i64) -> Option<Vec<f32>> {
        let rate = MODEL_RATE as f64;
        let mut own: Vec<&SpeakerTurn> = turns.iter().filter(|t| t.speaker == speaker).collect();
        own.sort_by(|a, b| (b.end - b.start).total_cmp(&(a.end - a.start)));
        let mut sum: Vec<f32> = Vec::new();
        let mut count = 0;
        for turn in own.iter().filter(|t| t.end - t.start >= 1.0).take(12) {
            let start = (turn.start * rate) as usize;
            let end = ((turn.start + (turn.end - turn.start).min(12.0)) * rate) as usize;
            if let Some(embedding) = self.embedding(&samples[start.min(samples.len())..end.min(samples.len())]) {
                if sum.is_empty() {
                    sum = vec![0.0; embedding.len()];
                }
                if embedding.len() == sum.len() {
                    sum.iter_mut().zip(&embedding).for_each(|(s, e)| *s += e);
                    count += 1;
                }
            }
        }
        if count == 0 {
            // Only short turns: one embedding over all of them.
            let joined: Vec<f32> = own
                .iter()
                .flat_map(|t| {
                    let start = ((t.start * rate) as usize).min(samples.len());
                    let end = ((t.end * rate) as usize).min(samples.len());
                    samples[start..end].iter().copied()
                })
                .collect();
            sum = self.embedding(&joined)?;
        }
        Some(normalized(sum))
    }

    /// The voice fingerprint of a stretch of speech (L2-normalized).
    pub fn embedding(&self, samples: &[f32]) -> Option<Vec<f32>> {
        if samples.len() < MODEL_RATE as usize / 2 {
            return None;
        }
        let stream = self.extractor.create_stream()?;
        stream.accept_waveform(MODEL_RATE as i32, samples);
        stream.input_finished();
        if !self.extractor.is_ready(&stream) {
            return None;
        }
        self.extractor.compute(&stream).map(normalized)
    }
}

fn normalized(vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        vector.into_iter().map(|x| x / norm).collect()
    } else {
        vector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_are_merged_and_padded() {
        let s = |seconds: f64| (seconds * 16_000.0) as usize;
        let segments = [(s(1.0), s(3.0)), (s(3.5), s(5.0)), (s(10.0), s(12.0))];
        let chunks = merge_segments(&segments, s(20.0));
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], (s(0.7), s(5.3)));
        assert_eq!(chunks[1], (s(9.7), s(12.3)));
    }

    #[test]
    fn chunks_stay_short() {
        let s = |seconds: f64| (seconds * 16_000.0) as usize;
        let segments: Vec<(usize, usize)> = (0..20).map(|i| (s(i as f64 * 5.0), s(i as f64 * 5.0 + 4.5))).collect();
        let chunks = merge_segments(&segments, s(100.0));
        assert!(chunks.iter().all(|(a, b)| b - a <= s(MAX_CHUNK_SECONDS + 2.0 * PADDING_SECONDS)));
        // Chunks never overlap.
        assert!(chunks.windows(2).all(|w| w[0].1 <= w[1].0));
    }
}
