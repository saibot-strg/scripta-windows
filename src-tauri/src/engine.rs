//! Speech to text (Parakeet TDT 0.6B v3 through sherpa-onnx) and speaker
//! diarization (pyannote segmentation 3.0 + NVIDIA TitaNet small), on the
//! processor or a graphics card.
//!
//! Long audio is cut at pauses (Silero VAD) into chunks of up to ~25 s and
//! transcribed chunk by chunk, so memory stays flat and progress is real.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use sherpa_onnx::{
    FastClusteringConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineSpeakerDiarization,
    OfflineSpeakerDiarizationConfig, OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    OfflineStream, OfflineTransducerModelConfig, SileroVadModelConfig, SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig,
    VadModelConfig, VoiceActivityDetector,
};

use crate::audio::MODEL_RATE;
use crate::gpu::Device;
use crate::model::WordTiming;
use crate::models::ModelKind;
use crate::transcript::{consolidate, consolidate_to, words_from_tokens, Diarization, SpeakerTurn, TokenTiming};

/// ONNX Runtime threads: half the cores, so the PC stays usable.
pub fn thread_count() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores / 2).clamp(2, 8) as i32
}

/// Logical cores that work spread over several threads leaves to the window
/// and other programs (with all of them busy the PC stopped responding).
const FREE_CORES: usize = 2;

/// Threads for work spread over the processor.
fn thread_budget() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    cores.saturating_sub(FREE_CORES).max(1)
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
    /// The voice activity detector's model.
    vad: Option<String>,
    /// Chunks decoded together in one call.
    batch: usize,
}

/// Longest chunk handed to the recognizer.
const MAX_CHUNK_SECONDS: f64 = 25.0;
/// Pauses shorter than this inside a chunk are kept (context for the model).
const MERGE_GAP_SECONDS: f64 = 1.0;
/// Audio added before and after each chunk, so word edges aren't cut.
const PADDING_SECONDS: f64 = 0.3;
/// Silence appended to each chunk before decoding (0.5 s).
const TAIL_PADDING: usize = MODEL_RATE as usize / 2;
/// Chunks per call on a graphics card. Each call costs at least ~0.15 s
/// there (measured on a Radeon RX 7900 XTX), however short the chunk, while
/// 25 s took 0.12 s; most chunks of a meeting are a few seconds long. 10 min
/// of a Teams meeting: 13.4 s one by one, 2.0 s in batches of 16. Not on the
/// processor: the int8 model quantizes over the whole batch, and its words
/// changed by a fifth.
const GPU_BATCH: usize = 16;

impl SpeechEngine {
    pub fn load(device: Device) -> Result<Self> {
        let kind = ModelKind::Speech.on(device);
        let (encoder, decoder, joiner) = match kind {
            ModelKind::SpeechFull => ("encoder.onnx", "decoder.onnx", "joiner.onnx"),
            _ => ("encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"),
        };
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer =
            OfflineTransducerModelConfig { encoder: path(kind, encoder), decoder: path(kind, decoder), joiner: path(kind, joiner) };
        config.model_config.tokens = path(kind, "tokens.txt");
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = thread_count();
        config.model_config.provider = Some(device.provider());
        config.decoding_method = Some("greedy_search".into());
        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| anyhow!("the speech model could not be loaded"))?;
        let batch = if device == Device::Cpu { 1 } else { GPU_BATCH };
        Ok(Self { recognizer, vad: path(kind, "silero_vad.onnx"), batch })
    }

    /// Words with timings in 16 kHz mono `samples`. `progress` gets 0...1.
    pub fn transcribe(&self, samples: &[f32], cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<Vec<WordTiming>> {
        // The voice detectors take about 7 s for 2 hours, a quarter of the
        // speech itself on the graphics card (RX 7900 XTX).
        let detection = if self.batch > 1 { 0.2 } else { 0.05 };
        let started = Instant::now();
        let mut chunks = speech_chunks(samples, self.vad.clone(), cancel, &mut |f| progress(detection * f))?;
        let detected = started.elapsed().as_secs_f64();
        // Batches of similar length waste less padding; the words are put
        // back in order at the end.
        if self.batch > 1 {
            chunks.sort_by_key(|(start, end)| end - start);
        }
        let total = chunks.iter().map(|(start, end)| end - start).sum::<usize>().max(1) as f64;
        let mut done = 0;
        let mut words = Vec::new();
        for batch in chunks.chunks(self.batch) {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled.into());
            }
            let parts: Vec<(&[f32], f64)> =
                batch.iter().map(|&(start, end)| (&samples[start..end], start as f64 / MODEL_RATE as f64)).collect();
            words.extend(self.transcribe_batch(&parts));
            done += batch.iter().map(|(start, end)| end - start).sum::<usize>();
            progress(detection + (1.0 - detection) * done as f64 / total);
        }
        words.sort_by(|a, b| a.start.total_cmp(&b.start));
        tracing::info!(
            chunks = chunks.len(),
            detection_seconds = detected,
            recognition_seconds = started.elapsed().as_secs_f64() - detected,
            "speech recognized"
        );
        progress(1.0);
        Ok(words)
    }

    /// Short audio in memory (no chunking), e.g. a test or dictation.
    pub fn transcribe_short(&self, samples: &[f32]) -> Vec<WordTiming> {
        self.transcribe_batch(&[(samples, 0.0)])
    }

    /// Chunks (samples, start in seconds) decoded in one call; sherpa-onnx
    /// pads them into one batch and masks the padding.
    fn transcribe_batch(&self, chunks: &[(&[f32], f64)]) -> Vec<WordTiming> {
        // Parakeet needs at least ~0.3 s.
        let chunks: Vec<&(&[f32], f64)> = chunks.iter().filter(|(samples, _)| samples.len() >= MODEL_RATE as usize * 3 / 10).collect();
        let streams: Vec<OfflineStream> = chunks
            .iter()
            .map(|(samples, _)| {
                // Transducers emit the last tokens only with some audio after them.
                let mut padded = Vec::with_capacity(samples.len() + TAIL_PADDING);
                padded.extend_from_slice(samples);
                padded.resize(samples.len() + TAIL_PADDING, 0.0);
                let stream = self.recognizer.create_stream();
                stream.accept_waveform(MODEL_RATE as i32, &padded);
                stream
            })
            .collect();
        match streams.as_slice() {
            [] => return Vec::new(),
            [single] => self.recognizer.decode(single),
            _ => self.recognizer.decode_multiple_streams(&streams.iter().collect::<Vec<_>>()),
        }
        streams.iter().zip(&chunks).flat_map(|(stream, (_, offset))| words_of(stream, *offset)).collect()
    }
}

/// The words of a decoded stream, `offset` seconds into the audio.
fn words_of(stream: &OfflineStream, offset: f64) -> Vec<WordTiming> {
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

/// Voice detectors run at once on the parts of long audio (one takes about
/// 28 s for 2 hours); each part is at least `DETECTION_PART_SECONDS` long.
const DETECTION_PARTS: usize = 4;
const DETECTION_PART_SECONDS: usize = 300;

/// Sample ranges with speech, merged into chunks of up to 25 s and padded.
/// Speech across the border of two parts comes back as two segments that
/// `merge_segments` joins again (or cuts, like the detector's own 20 s limit).
fn speech_chunks(
    samples: &[f32],
    vad_model: Option<String>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Vec<(usize, usize)>> {
    let parts = DETECTION_PARTS.min(thread_budget()).min(samples.len() / (DETECTION_PART_SECONDS * MODEL_RATE as usize)).max(1);
    // Whole windows per part, as if one detector read it all.
    let part_length = samples.len().div_ceil(parts).div_ceil(512) * 512;
    let mut segments = Vec::new();
    std::thread::scope(|scope| -> Result<()> {
        let others: Vec<_> = (1..parts)
            .map(|part| {
                let offset = part * part_length;
                let audio = &samples[offset.min(samples.len())..(offset + part_length).min(samples.len())];
                let vad_model = vad_model.clone();
                scope.spawn(move || detect_speech(audio, offset, vad_model, cancel, &mut |_| {}))
            })
            .collect();
        // The first part reports progress; the others keep pace.
        segments = detect_speech(&samples[..part_length.min(samples.len())], 0, vad_model.clone(), cancel, progress)?;
        for other in others {
            segments.extend(other.join().map_err(|_| anyhow!("the voice detector failed"))??);
        }
        Ok(())
    })?;
    Ok(merge_segments(&segments, samples.len()))
}

/// Speech segments in one part of the audio that starts at `offset`.
fn detect_speech(
    samples: &[f32],
    offset: usize,
    vad_model: Option<String>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Vec<(usize, usize)>> {
    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: vad_model,
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
            let start = offset + segment.start().max(0) as usize;
            segments.push((start, start + segment.n().max(0) as usize));
            vad.pop();
        }
    };
    for (index, window) in samples.chunks(512).enumerate() {
        // About every 5 s of audio.
        if index % 160 == 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled.into());
            }
            progress((index * 512) as f64 / samples.len().max(1) as f64);
        }
        vad.accept_waveform(window);
        collect(&vad);
    }
    vad.flush();
    collect(&vad);
    Ok(segments)
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
    /// For recordings over one piece (`LONG_WINDOW_SHIFT`); each worker
    /// thread makes its own instance from it.
    long_config: OfflineSpeakerDiarizationConfig,
}

/// Pieces of a long recording diarized at the same time, each by its own
/// sherpa-onnx instance on its own share of the processor's threads.
const DIARIZATION_WORKERS: usize = 3;

/// Longest stretch diarized at once. sherpa-onnx clusters all voice
/// embeddings of the audio in one distance matrix, which grows with the
/// square of the length: for the 2-hour meeting the process peaked at 27 GB.
const PIECE_SECONDS: f64 = 600.0;

/// Samples per piece: the audio cut into equal pieces of at most 10 minutes.
fn piece_length(samples: usize) -> usize {
    let most = (PIECE_SECONDS * MODEL_RATE as f64) as usize;
    samples.div_ceil(samples.div_ceil(most).max(1)).max(1)
}

/// Clustering threshold for an unknown number of speakers (larger means
/// fewer speakers). With TitaNet small, synthetic dialogues (German and
/// English, clean and call-like) separated correctly from 0.4 to 0.6. On real
/// calls sherpa-onnx's complete linkage splits each person into many parts
/// (517 in a 2-hour Teams meeting); `transcript::consolidate` joins them.
pub const CLUSTER_THRESHOLD: f32 = 0.5;

/// How far the segmentation's 10-second window moves, as a share of it, in
/// recordings longer than one piece. sherpa-onnx's default 0.1 analyses every
/// second ten times. The 2-hour Teams meeting, per track: 0.1 took 241 s,
/// 0.25 took 104 s with the same speakers (6 and 1), 0.5 took 59 s but merged
/// two of the three main voices. Short recordings keep 0.1: with 0.25 the test
/// dialogues (under 2 minutes) lost voices, and they take seconds anyway.
pub const LONG_WINDOW_SHIFT: f32 = 0.25;
const SHORT_WINDOW_SHIFT: f32 = 0.1;

impl SpeakerEngine {
    pub fn load(device: Device) -> Result<Self> {
        Self::load_with(&ModelKind::Speakers.file("titanet-small.onnx"), CLUSTER_THRESHOLD, LONG_WINDOW_SHIFT, device)
    }

    /// With another embedding model, threshold and window shift for long
    /// recordings (calibration).
    pub fn load_with(embedding_model: &std::path::Path, threshold: f32, long_window_shift: f32, device: Device) -> Result<Self> {
        let kind = ModelKind::Speakers;
        let provider = device.provider();
        let embedding = SpeakerEmbeddingExtractorConfig {
            model: Some(embedding_model.to_string_lossy().into_owned()),
            num_threads: thread_count(),
            debug: false,
            provider: Some(provider.clone()),
        };
        let config = OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                    model: path(kind, "segmentation.onnx"),
                    window_shift_ratio: SHORT_WINDOW_SHIFT,
                },
                num_threads: thread_count(),
                provider: Some(provider),
                ..Default::default()
            },
            embedding: embedding.clone(),
            clustering: FastClusteringConfig { num_clusters: -1, threshold, ..Default::default() },
            min_duration_on: 0.3,
            min_duration_off: 0.5,
        };
        let diarizer =
            OfflineSpeakerDiarization::create(&config).ok_or_else(|| anyhow!("the speaker model could not be loaded"))?;
        let mut long_config = config.clone();
        long_config.segmentation.pyannote.window_shift_ratio = long_window_shift;
        let extractor =
            SpeakerEmbeddingExtractor::create(&embedding).ok_or_else(|| anyhow!("the voice model could not be loaded"))?;
        Ok(Self { diarizer, extractor, config, long_config })
    }

    /// Who spoke when in 16 kHz mono `samples`, exactly `speaker_count`
    /// speakers if given, plus one voice fingerprint per speaker.
    pub fn diarize(&self, samples: &[f32], speaker_count: Option<usize>) -> Result<Diarization> {
        self.diarize_cancellable(samples, speaker_count, &AtomicBool::new(false), |_| {})
    }

    /// `diarize`, stopping between pieces once `cancel` is set; `progress`
    /// gets 0...1.
    pub fn diarize_cancellable(
        &self,
        samples: &[f32],
        speaker_count: Option<usize>,
        cancel: &AtomicBool,
        mut progress: impl FnMut(f64),
    ) -> Result<Diarization> {
        let mut config = self.config.clone();
        match speaker_count {
            // A number the user asked for is kept as it is.
            Some(count) if samples.len() <= piece_length(samples.len()) => {
                config.clustering.num_clusters = count as i32;
                return self.run(samples, &config, cancel, &mut progress);
            }
            Some(count) => return Ok(consolidate_to(self.run(samples, &config, cancel, &mut progress)?, count)),
            None => {}
        }
        let diarization = self.run(samples, &config, cancel, &mut progress)?;
        let found = diarization.speakers().len();
        let consolidated = consolidate(diarization);
        if consolidated.speakers().len() != found {
            tracing::info!(found, kept = consolidated.speakers().len(), "speakers consolidated");
        }
        Ok(consolidated)
    }

    /// Like `diarize` with another clustering threshold and without the
    /// consolidation (calibration).
    pub fn diarize_raw(&self, samples: &[f32], threshold: f32) -> Result<Diarization> {
        let mut config = self.config.clone();
        config.clustering.threshold = threshold;
        self.run(samples, &config, &AtomicBool::new(false), &mut |_| {})
    }

    /// The raw clustering, piece by piece; each piece's speakers are numbered
    /// apart, `transcript::consolidate` joins them across the recording.
    fn run(
        &self,
        samples: &[f32],
        config: &OfflineSpeakerDiarizationConfig,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(f64),
    ) -> Result<Diarization> {
        let started = Instant::now();
        let length = piece_length(samples.len());
        let pieces: Vec<&[f32]> = samples.chunks(length).collect();
        let results = if pieces.len() > 1 {
            self.run_pieces(&pieces, config, cancel, &mut |f| progress(0.85 * f))?
        } else {
            self.diarizer.set_config(config);
            let result = self.diarizer.process(samples).ok_or_else(|| anyhow!("speaker separation failed"))?;
            vec![result.sort_by_start_time().into_iter().map(|s| (s.start, s.end, s.speaker)).collect()]
        };
        let segmentation = started.elapsed().as_secs_f64();
        let mut numbers: HashMap<(usize, i32), i64> = HashMap::new();
        let mut turns = Vec::new();
        for (index, segments) in results.into_iter().enumerate() {
            let offset = (index * length) as f64 / MODEL_RATE as f64;
            for (start, end, speaker) in segments {
                if end <= start {
                    continue;
                }
                let next = numbers.len() as i64 + 1;
                let speaker = *numbers.entry((index, speaker)).or_insert(next);
                turns.push(SpeakerTurn { speaker, start: offset + start as f64, end: offset + end as f64 });
            }
        }
        progress(0.85);
        let diarization = self.refresh_voices(samples, Diarization { turns, embeddings: HashMap::new() });
        tracing::info!(
            pieces = pieces.len(),
            parts = diarization.speakers().len(),
            segmentation,
            voices = started.elapsed().as_secs_f64() - segmentation,
            "speaker parts"
        );
        progress(1.0);
        Ok(diarization)
    }

    /// The pieces of a long recording on `DIARIZATION_WORKERS` threads; per
    /// piece the segments (start and end in seconds, speaker within the piece).
    fn run_pieces(
        &self,
        pieces: &[&[f32]],
        config: &OfflineSpeakerDiarizationConfig,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(f64),
    ) -> Result<Vec<Vec<(f32, f32, i32)>>> {
        let budget = thread_budget();
        let workers = DIARIZATION_WORKERS.min(pieces.len()).min(budget);
        let threads = (budget / workers).max(1) as i32;
        let mut worker_config = self.long_config.clone();
        worker_config.clustering = config.clustering.clone();
        worker_config.segmentation.num_threads = threads;
        worker_config.embedding.num_threads = threads;
        let next = AtomicUsize::new(0);
        let mut results = vec![Vec::new(); pieces.len()];
        std::thread::scope(|scope| -> Result<()> {
            let (sender, receiver) = mpsc::channel();
            for _ in 0..workers {
                let sender = sender.clone();
                let (next, worker_config) = (&next, &worker_config);
                scope.spawn(move || {
                    let Some(diarizer) = OfflineSpeakerDiarization::create(worker_config) else {
                        let _ = sender.send(None);
                        return;
                    };
                    while !cancel.load(Ordering::Relaxed) {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(piece) = pieces.get(index) else { break };
                        let segments = diarizer
                            .process(piece)
                            .map(|r| r.sort_by_start_time().into_iter().map(|s| (s.start, s.end, s.speaker)).collect::<Vec<_>>());
                        if sender.send(segments.map(|s| (index, s))).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(sender);
            let mut done = 0;
            for message in receiver {
                let (index, segments) = message.ok_or_else(|| anyhow!("speaker separation failed"))?;
                results[index] = segments;
                done += 1;
                progress(done as f64 / pieces.len() as f64);
            }
            Ok(())
        })?;
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled.into());
        }
        Ok(results)
    }

    /// The diarization with each speaker's voice computed from their longest
    /// turns. (Doing this again after merging, for a second pass, didn't help:
    /// over a Teams call it made three different people 0.63 to 0.75 alike.)
    fn refresh_voices(&self, samples: &[f32], diarization: Diarization) -> Diarization {
        let embeddings = diarization
            .speakers()
            .into_iter()
            .filter_map(|speaker| self.voice(samples, &diarization.turns, speaker).map(|voice| (speaker, voice)))
            .collect();
        Diarization { embeddings, ..diarization }
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
    fn long_audio_is_diarized_in_equal_pieces() {
        let minutes = |m: usize| m * 60 * 16_000;
        assert_eq!(piece_length(minutes(7)), minutes(7));
        assert_eq!(piece_length(minutes(10)), minutes(10));
        assert_eq!(piece_length(minutes(25)), minutes(25).div_ceil(3));
        assert_eq!(piece_length(minutes(118)), minutes(118).div_ceil(12));
        assert_eq!(piece_length(0), 1);
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
