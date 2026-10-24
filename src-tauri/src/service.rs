//! Transcription queue: runs one job at a time on its own thread, keeps the
//! models loaded while there is work and writes results into the library.
//! A port of Scripta's `TranscriptionService`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::{mf, MODEL_RATE};
use crate::engine::{EngineError, SpeakerEngine, SpeechEngine};
use crate::gpu::{self, Device};
use crate::model::{TrackRole, TranscriptionStatus, WordTiming};
use crate::models::{ModelKind, ENGINE_IDENTIFIER};
use crate::priority;
use crate::state::{tr, AppState};
use crate::storage::Storage;
use crate::transcript::{self, Diarization, DraftTranscript};
use crate::voices::LabelVoice;

/// The loaded models take about 1 GB; free them after this idle time.
const IDLE_UNLOAD: Duration = Duration::from_secs(300);

/// Share of a track's progress taken by reading and transcription when
/// diarization follows (2-hour Teams track on the test PC: 50 s against 60 s
/// with the graphics card, about 140 s against 60 s on the processor).
fn transcription_share(device: Device) -> f64 {
    match device {
        Device::Gpu(_) => 0.45,
        Device::Cpu => 0.7,
    }
}

/// The window redraws on every progress event, and decoding alone reports
/// each audio frame (about 340,000 for 2 hours): send one at most this often
/// and only when the value moved visibly.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const PROGRESS_STEP: f64 = 0.005;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ModelState {
    NotInstalled,
    /// On disk, not loaded into memory.
    Installed,
    Downloading { fraction: f64 },
    Loading,
    Ready,
    Failed { message: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSnapshot {
    pub speech: ModelState,
    pub speakers: ModelState,
    /// Recording IDs waiting or running, in order.
    pub queue: Vec<String>,
    pub running: Option<String>,
    /// Progress (0...1) of the running job.
    pub progress: f64,
}

struct Job {
    id: String,
    speaker_count: Option<usize>,
}

struct Shared {
    speech: ModelState,
    speakers: ModelState,
    queue: VecDeque<Job>,
    running: Option<String>,
    progress: f64,
    /// When and with which value the last progress event went out.
    reported: (Instant, f64),
    cancel: Arc<AtomicBool>,
}

enum Command {
    Wake,
    Prepare(ModelKind),
    Delete(ModelKind),
}

#[derive(Clone)]
pub struct TranscriptionService {
    shared: Arc<Mutex<Shared>>,
    commands: Sender<Command>,
}

fn initial_state(kind: ModelKind) -> ModelState {
    if kind.is_installed() {
        ModelState::Installed
    } else {
        ModelState::NotInstalled
    }
}

impl TranscriptionService {
    pub fn start(app: AppHandle) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            speech: initial_state(ModelKind::Speech),
            speakers: initial_state(ModelKind::Speakers),
            queue: VecDeque::new(),
            running: None,
            progress: 0.0,
            reported: (Instant::now(), 0.0),
            cancel: Arc::new(AtomicBool::new(false)),
        }));
        let (commands, receiver) = mpsc::channel();
        let service = Self { shared: shared.clone(), commands };
        let worker = Worker { app, shared, speech: None, speakers: None, device: Device::Cpu, last_used: Instant::now() };
        std::thread::Builder::new()
            .name("transcription".into())
            .spawn(move || worker.run(receiver))
            .expect("transcription thread");
        service
    }

    pub fn snapshot(&self) -> ServiceSnapshot {
        let shared = self.shared.lock().unwrap();
        ServiceSnapshot {
            speech: shared.speech.clone(),
            speakers: shared.speakers.clone(),
            queue: shared.queue.iter().map(|j| j.id.clone()).collect(),
            running: shared.running.clone(),
            progress: shared.progress,
        }
    }

    pub fn is_queued(&self, id: &str) -> bool {
        self.shared.lock().unwrap().queue.iter().any(|j| j.id == id)
    }

    /// `speaker_count`: the number of voices on the system track (meetings)
    /// or in the recording, when the user corrects a wrong separation.
    /// The caller sets the recording's status to queued.
    pub fn enqueue(&self, id: &str, speaker_count: Option<usize>) {
        {
            let mut shared = self.shared.lock().unwrap();
            if shared.queue.iter().any(|j| j.id == id) {
                return;
            }
            shared.queue.push_back(Job { id: id.to_string(), speaker_count });
        }
        let _ = self.commands.send(Command::Wake);
    }

    /// Removes a recording from the queue or stops its running job. Returns
    /// true if it was waiting (the caller restores its status).
    pub fn cancel(&self, id: &str) -> bool {
        let mut shared = self.shared.lock().unwrap();
        if shared.running.as_deref() == Some(id) {
            shared.cancel.store(true, Ordering::Relaxed);
            return false;
        }
        let before = shared.queue.len();
        shared.queue.retain(|j| j.id != id);
        before != shared.queue.len()
    }

    /// Downloads (if needed) and loads a model, e.g. from the settings.
    pub fn prepare(&self, kind: ModelKind) {
        let _ = self.commands.send(Command::Prepare(kind));
    }

    pub fn delete_model(&self, kind: ModelKind) {
        let _ = self.commands.send(Command::Delete(kind));
    }

    pub fn is_busy(&self) -> bool {
        let shared = self.shared.lock().unwrap();
        shared.running.is_some()
            || matches!(shared.speech, ModelState::Downloading { .. } | ModelState::Loading)
            || matches!(shared.speakers, ModelState::Downloading { .. } | ModelState::Loading)
    }
}

struct Worker {
    app: AppHandle,
    shared: Arc<Mutex<Shared>>,
    speech: Option<SpeechEngine>,
    speakers: Option<SpeakerEngine>,
    /// Where the loaded models run.
    device: Device,
    last_used: Instant,
}

/// Where an audio file comes from; decides how speakers are labeled.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum TrackKind {
    Microphone,
    System,
    /// A recording without separate tracks (microphone recording, import).
    Single,
}

struct TrackResult {
    words: Vec<WordTiming>,
    speakers: Option<Diarization>,
}

impl Worker {
    fn run(mut self, receiver: mpsc::Receiver<Command>) {
        // The settings live in the app state, managed right after the service
        // starts; they decide which speech model the status shows.
        while self.app.try_state::<AppState>().is_none() {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.select_device();
        loop {
            let next = {
                let mut shared = self.shared.lock().unwrap();
                let job = shared.queue.front().map(|j| (j.id.clone(), j.speaker_count));
                if let Some((id, _)) = &job {
                    shared.running = Some(id.clone());
                    shared.progress = 0.0;
                    shared.reported = (Instant::now(), 0.0);
                    shared.cancel = Arc::new(AtomicBool::new(false));
                }
                job
            };
            if let Some((id, speaker_count)) = next {
                self.emit_queue();
                self.process(&id, speaker_count);
                self.last_used = Instant::now();
                {
                    let mut shared = self.shared.lock().unwrap();
                    shared.running = None;
                    shared.progress = 0.0;
                    if shared.queue.front().is_some_and(|j| j.id == id) {
                        shared.queue.pop_front();
                    }
                }
                self.emit_queue();
                // Commands that arrived meanwhile (e.g. a model download).
                while let Ok(command) = receiver.try_recv() {
                    self.handle(command);
                }
                continue;
            }
            match receiver.recv_timeout(Duration::from_secs(30)) {
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => self.unload_if_idle(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Wake => {}
            Command::Prepare(kind) => {
                let _ = self.ensure(kind);
                self.last_used = Instant::now();
            }
            Command::Delete(kind) => {
                let files = kind.on(self.select_device());
                match kind {
                    ModelKind::Speakers => self.speakers = None,
                    _ => self.speech = None,
                }
                if let Err(error) = files.delete() {
                    tracing::error!(%error, "deleting model failed");
                }
                self.set_model(kind, initial_state(files));
            }
        }
    }

    fn unload_if_idle(&mut self) {
        if self.last_used.elapsed() < IDLE_UNLOAD || (self.speech.is_none() && self.speakers.is_none()) {
            return;
        }
        self.unload();
        tracing::info!("models unloaded after idle time");
    }

    fn unload(&mut self) {
        self.speech = None;
        self.speakers = None;
        for kind in [ModelKind::Speech, ModelKind::Speakers] {
            if self.model_state(kind) == ModelState::Ready {
                self.set_model(kind, ModelState::Installed);
            }
        }
    }

    /// The device the settings ask for speech recognition; switching it
    /// reloads the speech model (another one on the graphics card).
    fn select_device(&mut self) -> Device {
        let settings = self.app.state::<AppState>().settings();
        let wanted = Device::choose(settings.uses_gpu, &settings.gpu_name);
        if wanted != self.device {
            tracing::info!(card = wanted.card().map(|c| c.name), "speech recognition device");
            self.speech = None;
            self.device = wanted;
            if !matches!(self.model_state(ModelKind::Speech), ModelState::Downloading { .. } | ModelState::Loading) {
                self.set_model(ModelKind::Speech, initial_state(ModelKind::Speech.on(wanted)));
            }
        }
        wanted
    }

    fn model_state(&self, kind: ModelKind) -> ModelState {
        let shared = self.shared.lock().unwrap();
        match kind {
            ModelKind::Speakers => shared.speakers.clone(),
            _ => shared.speech.clone(),
        }
    }

    fn set_model(&self, kind: ModelKind, state: ModelState) {
        {
            let mut shared = self.shared.lock().unwrap();
            match kind {
                ModelKind::Speakers => shared.speakers = state,
                _ => shared.speech = state,
            }
        }
        self.emit_queue();
    }

    fn emit_queue(&self) {
        let snapshot = self.app.state::<AppState>().service.snapshot();
        let _ = self.app.emit("service-changed", snapshot);
    }

    /// Downloads (first run) and loads a model. Safe to call repeatedly.
    fn ensure(&mut self, kind: ModelKind) -> Result<()> {
        let device = self.select_device();
        // On the graphics card speaker separation was slower than on the
        // processor (5 min of a Teams meeting: 14 s against 10 s).
        let device = if kind == ModelKind::Speakers { Device::Cpu } else { device };
        let loaded = match kind {
            ModelKind::Speakers => self.speakers.is_some(),
            _ => self.speech.is_some(),
        };
        if loaded {
            return Ok(());
        }
        let files = kind.on(device);
        let result = (|| -> Result<()> {
            if !files.is_installed() {
                let cancel = AtomicBool::new(false);
                let mut last = Instant::now() - Duration::from_secs(1);
                self.set_model(kind, ModelState::Downloading { fraction: 0.0 });
                files.download(&cancel, |fraction| {
                    if last.elapsed() >= Duration::from_millis(250) {
                        last = Instant::now();
                        self.set_model(kind, ModelState::Downloading { fraction });
                    }
                })?;
            }
            self.set_model(kind, ModelState::Loading);
            let started = Instant::now();
            let _guard = gpu::Guard::new(device);
            match kind {
                ModelKind::Speakers => self.speakers = Some(SpeakerEngine::load(device)?),
                _ => self.speech = Some(SpeechEngine::load(device)?),
            }
            tracing::info!(model = ?files, ?device, seconds = started.elapsed().as_secs_f64(), "model loaded");
            Ok(())
        })();
        match &result {
            Ok(()) => self.set_model(kind, ModelState::Ready),
            Err(error) => {
                tracing::error!(?kind, %error, "model unavailable");
                self.set_model(kind, ModelState::Failed { message: error.to_string() });
            }
        }
        result
    }

    fn set_progress(&self, id: &str, fraction: f64) {
        {
            let mut shared = self.shared.lock().unwrap();
            shared.progress = fraction;
            let (at, value) = shared.reported;
            if at.elapsed() < PROGRESS_INTERVAL || (fraction - value).abs() < PROGRESS_STEP {
                return;
            }
            shared.reported = (Instant::now(), fraction);
        }
        let _ = self.app.emit("transcription-progress", serde_json::json!({ "id": id, "fraction": fraction }));
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.shared.lock().unwrap().cancel.clone()
    }

    fn process(&mut self, id: &str, speaker_count: Option<usize>) {
        let app = self.app.clone();
        let state = app.state::<AppState>();
        // A recording that is still being encoded to AAC: wait for the final file.
        let waited = Instant::now();
        while state.is_encoding(id) && waited.elapsed() < Duration::from_secs(600) {
            std::thread::sleep(Duration::from_millis(250));
        }
        let inputs = match self.inputs(id) {
            Ok(inputs) => inputs,
            Err(error) => {
                self.fail(id, &error.to_string());
                return;
            }
        };

        state.set_status(id, TranscriptionStatus::Running, None);
        let _background = priority::Background::new();
        let settings = state.settings();
        let separates = settings.separates_speakers;
        let cancel = self.cancel_flag();
        let started = Instant::now();

        let result = (|| -> Result<HashMap<TrackKind, TrackResult>> {
            self.ensure(ModelKind::Speech).map_err(|e| anyhow!(tr("Das Spracherkennungsmodell konnte nicht geladen werden.", "The speech recognition model could not be loaded.") + " " + &e.to_string()))?;
            let mut tracks: HashMap<TrackKind, TrackResult> = HashMap::new();
            let count = inputs.len() as f64;
            // The next track is read while this one is transcribed.
            let mut next: Option<JoinHandle<Result<Vec<f32>>>> = None;
            for (index, (path, kind)) in inputs.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(EngineError::Cancelled.into());
                }
                let wants_speakers = separates
                    && (*kind != TrackKind::Microphone
                        || settings.separates_room_speakers
                        || tracks.get(&TrackKind::System).map(|t| t.words.len() < 20).unwrap_or(true));
                let share = if wants_speakers { transcription_share(self.device) } else { 1.0 };
                let report = |fraction: f64| (index as f64 + fraction) / count;

                let samples = match next.take() {
                    Some(reading) => reading.join().map_err(|_| anyhow!("reading the audio failed"))??,
                    None => read_audio(path, &cancel, |f| self.set_progress(id, report(f * 0.1 * share)))?,
                };
                if let Some((path, _)) = inputs.get(index + 1) {
                    let (path, cancel) = (path.clone(), cancel.clone());
                    next = Some(std::thread::spawn(move || read_audio(&path, &cancel, |_| {})));
                }
                if samples.len() < MODEL_RATE as usize / 2 {
                    tracks.insert(*kind, TrackResult { words: Vec::new(), speakers: None });
                    continue;
                }
                let engine = self.speech.as_ref().ok_or_else(|| anyhow!("speech model missing"))?;
                let words = {
                    let _guard = gpu::Guard::new(self.device);
                    engine.transcribe(&samples, &cancel, |f| self.set_progress(id, report((0.1 + 0.9 * f) * share)))?
                };
                let speakers = if wants_speakers && !words.is_empty() {
                    let count_for_track = if *kind == TrackKind::Microphone { None } else { speaker_count };
                    self.diarize(&samples, count_for_track, &cancel, id, |f| report(share + (1.0 - share) * f))?
                } else {
                    None
                };
                self.set_progress(id, report(1.0));
                tracks.insert(*kind, TrackResult { words, speakers });
            }
            Ok(tracks)
        })();

        match result {
            Ok(tracks) => {
                let draft = if let (Some(me), Some(others)) = (tracks.get(&TrackKind::Microphone), tracks.get(&TrackKind::System)) {
                    transcript::meeting_transcript(&me.words, me.speakers.as_ref(), &others.words, others.speakers.as_ref())
                } else if let Some(single) = tracks.get(&TrackKind::Single) {
                    transcript::transcript(&single.words, single.speakers.as_ref())
                } else {
                    DraftTranscript::default()
                };
                if let Err(error) = self.store(id, &draft) {
                    self.fail(id, &error.to_string());
                    return;
                }
                tracing::info!(
                    segments = draft.segments.len(),
                    device = ?self.device,
                    seconds = started.elapsed().as_secs_f64(),
                    "transcribed"
                );
            }
            Err(error) if error.downcast_ref::<EngineError>().is_some() => {
                let has_transcript = state.library.lock().unwrap().has_transcript(id).unwrap_or(false);
                let status = if has_transcript { TranscriptionStatus::Done } else { TranscriptionStatus::NotTranscribed };
                state.set_status(id, status, None);
            }
            Err(error) => self.fail(id, &error.to_string()),
        }
    }

    /// Diarization is a bonus: if it fails, the transcript is still stored,
    /// just without separate speakers.
    fn diarize(
        &mut self,
        samples: &[f32],
        speaker_count: Option<usize>,
        cancel: &AtomicBool,
        id: &str,
        to_job: impl Fn(f64) -> f64,
    ) -> Result<Option<Diarization>> {
        if self.ensure(ModelKind::Speakers).is_err() {
            return Ok(None);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled.into());
        }
        let Some(engine) = self.speakers.as_ref() else { return Ok(None) };
        let started = Instant::now();
        match engine.diarize_cancellable(samples, speaker_count, cancel, |f| self.set_progress(id, to_job(f))) {
            Ok(diarization) => {
                tracing::info!(
                    speakers = diarization.speakers().len(),
                    audio_seconds = samples.len() as f64 / MODEL_RATE as f64,
                    seconds = started.elapsed().as_secs_f64(),
                    "diarized"
                );
                Ok(Some(diarization))
            }
            Err(error) if error.downcast_ref::<EngineError>().is_some() => Err(error),
            Err(error) => {
                tracing::error!(%error, "diarization failed");
                Ok(None)
            }
        }
    }

    /// The audio files to transcribe: meetings per track (system first:
    /// whether the microphone needs diarization depends on it).
    fn inputs(&self, id: &str) -> Result<Vec<(PathBuf, TrackKind)>> {
        let state = self.app.state::<AppState>();
        let library = state.library.lock().unwrap();
        let recording = library.recording(id)?.ok_or_else(|| anyhow!("recording not found"))?;
        let storage = Storage::for_root(recording.storage_root.as_deref());
        let tracks = library.tracks(id)?;
        let file = |role: TrackRole| {
            tracks.iter().find(|t| t.role == role).map(|t| storage.file(id, &t.file_name)).filter(|p| p.exists())
        };
        if let (Some(microphone), Some(system)) = (file(TrackRole::Microphone), file(TrackRole::System)) {
            return Ok(vec![(system, TrackKind::System), (microphone, TrackKind::Microphone)]);
        }
        match file(TrackRole::Primary) {
            Some(primary) => Ok(vec![(primary, TrackKind::Single)]),
            None => Err(anyhow!(tr("Die Audiodatei dieser Aufnahme fehlt.", "The audio file of this recording is missing."))),
        }
    }

    /// Replaces the recording's transcript and names the speakers whose
    /// voices are recognized.
    fn store(&self, id: &str, draft: &DraftTranscript) -> Result<()> {
        let state = self.app.state::<AppState>();
        let text: String = draft.segments.iter().map(|s| s.text()).collect::<Vec<_>>().join(" ");
        let language = transcript::detect_language(&text);
        let recognizes = state.settings().recognizes_voices;
        {
            let mut library = state.library.lock().unwrap();
            if !library.contains(id)? {
                // Deleted while transcribing.
                return Ok(());
            }
            let (speakers, old) = library.store_transcript(id, draft, language.as_deref(), ENGINE_IDENTIFIER)?;
            let mut voices = state.voices.lock().unwrap();
            voices.forget(&old)?;
            let label_voices: Vec<LabelVoice> = speakers
                .iter()
                .filter_map(|(key, label)| {
                    draft.voices.get(key).map(|embedding| LabelVoice {
                        label_id: label.clone(),
                        embedding: embedding.clone(),
                        speech_seconds: transcript::speech_of(*key, &draft.segments),
                    })
                })
                .collect();
            for (label, name) in voices.recognize(&label_voices, id, recognizes)? {
                library.set_custom_name(&label, Some(&name))?;
            }
        }
        state.notify_library(Some(id));
        let _ = self.app.emit("voices-changed", ());
        Ok(())
    }

    fn fail(&self, id: &str, message: &str) {
        tracing::error!(recording = id, error = message, "transcription failed");
        self.app.state::<AppState>().set_status(id, TranscriptionStatus::Failed, Some(message));
    }
}

/// A track as 16 kHz mono samples; stops early once `cancel` is set.
fn read_audio(path: &Path, cancel: &AtomicBool, progress: impl FnMut(f64)) -> Result<Vec<f32>> {
    let started = Instant::now();
    let mut samples = Vec::new();
    mf::decode_mono(
        path,
        MODEL_RATE,
        |chunk| {
            samples.extend_from_slice(chunk);
            Ok(!cancel.load(Ordering::Relaxed))
        },
        progress,
    )
    .map_err(|e| anyhow!(tr("Die Audiodatei konnte nicht gelesen werden.", "The audio file could not be read.") + " " + &e.to_string()))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled.into());
    }
    tracing::info!(
        file = ?path.file_name(),
        audio_seconds = samples.len() as f64 / MODEL_RATE as f64,
        seconds = started.elapsed().as_secs_f64(),
        "audio read"
    );
    Ok(samples)
}
