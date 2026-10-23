//! Recording: microphone recordings and meetings (microphone "Ich" plus
//! system or app audio "Andere" as separate, time-aligned tracks).
//!
//! Captured audio goes to disk immediately (crash-safe WAV). After stopping,
//! the microphone is levelled, a meeting's tracks are mixed for playback,
//! and the recording is added to the library.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Local;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::capture::{now_100ns, Capture, CaptureSource};
use crate::audio::wav::TrackSink;
use crate::audio::{devices, leveler, mixer, wav};
use crate::format;
use crate::model::RecordingSource;
use crate::state::{tr, AppState};
use crate::storage::{RecordingManifest, Storage, MICROPHONE_FILE, MINIMUM_FREE_SPACE, MIX_FILE, RECORDING_FILE, SYSTEM_FILE};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecorderState {
    Idle,
    Recording,
    Paused,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub state: RecorderState,
    pub is_meeting: bool,
    pub duration: f64,
    /// 0...1 meter levels.
    pub microphone_level: f32,
    pub system_level: f32,
    /// Stopped recordings still being finished (levelling, mixing).
    pub saving: Vec<RecordingSource>,
    /// The app recorded in a meeting, if not all system audio.
    pub source_name: Option<String>,
    pub microphone_name: Option<String>,
}

/// Levels and duration, written by the capture threads.
#[derive(Default)]
struct Meters {
    microphone: AtomicU32,
    system: AtomicU32,
    /// When each level was last written (ms on the QPC clock).
    microphone_at: AtomicU64,
    system_at: AtomicU64,
    duration_ms: AtomicU64,
}

/// A level older than this reads as silence: loopback capture delivers
/// nothing while no app plays sound.
const LEVEL_MAX_AGE_MS: u64 = 300;

impl Meters {
    fn set(level: &AtomicU32, at: &AtomicU64, value: f32) {
        level.store(value.to_bits(), Ordering::Relaxed);
        at.store(now_100ns() / 10_000, Ordering::Relaxed);
    }

    fn get(level: &AtomicU32, at: &AtomicU64) -> f32 {
        if (now_100ns() / 10_000).saturating_sub(at.load(Ordering::Relaxed)) > LEVEL_MAX_AGE_MS {
            return 0.0;
        }
        f32::from_bits(level.load(Ordering::Relaxed))
    }
}

struct Active {
    manifest: RecordingManifest,
    storage: Storage,
    paused: bool,
    microphone_sink: Arc<Mutex<TrackSink>>,
    system_sink: Option<Arc<Mutex<TrackSink>>>,
    microphone: Option<Capture>,
    system: Option<Capture>,
    meters: Arc<Meters>,
    source_name: Option<String>,
    microphone_name: Option<String>,
    ticker: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct Recorder {
    active: Option<Active>,
    pub saving: Vec<RecordingSource>,
}

/// A stopped recording whose files are closed but not finished yet.
pub struct Finished {
    pub manifest: RecordingManifest,
    pub storage: Storage,
}

impl Recorder {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn active_id(&self) -> Option<String> {
        self.active.as_ref().map(|a| a.manifest.id.clone())
    }

    pub fn status(&self) -> RecorderStatus {
        match &self.active {
            Some(active) => RecorderStatus {
                state: if active.paused { RecorderState::Paused } else { RecorderState::Recording },
                is_meeting: active.manifest.is_meeting(),
                duration: active.meters.duration_ms.load(Ordering::Relaxed) as f64 / 1000.0,
                microphone_level: Meters::get(&active.meters.microphone, &active.meters.microphone_at),
                system_level: Meters::get(&active.meters.system, &active.meters.system_at),
                saving: self.saving.clone(),
                source_name: active.source_name.clone(),
                microphone_name: active.microphone_name.clone(),
            },
            None => RecorderStatus {
                state: RecorderState::Idle,
                is_meeting: false,
                duration: 0.0,
                microphone_level: 0.0,
                system_level: 0.0,
                saving: self.saving.clone(),
                source_name: None,
                microphone_name: None,
            },
        }
    }
}

pub fn emit_status(app: &AppHandle) {
    let status = app.state::<AppState>().recorder.lock().unwrap().status();
    let _ = app.emit("recorder-status", status);
}

/// Errors worth explaining to the user.
#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("not enough space")]
    NotEnoughSpace,
    #[error("microphone access denied")]
    MicrophoneDenied,
    #[error("{0}")]
    Other(String),
}

impl RecordingError {
    pub fn notice(&self) -> (String, Option<String>, Option<&'static str>) {
        match self {
            Self::NotEnoughSpace => (
                tr("Nicht genug Speicherplatz", "Not enough disk space"),
                Some(tr(
                    "Für eine Aufnahme braucht Scripta mindestens 1 GB freien Speicher am Speicherort.",
                    "Scripta needs at least 1 GB of free space at the storage location to record.",
                )),
                None,
            ),
            Self::MicrophoneDenied => (
                tr("Kein Zugriff auf das Mikrofon", "No access to the microphone"),
                Some(tr(
                    "Erlaube in den Windows-Einstellungen unter Datenschutz › Mikrofon den Zugriff für Desktop-Apps.",
                    "Allow microphone access for desktop apps in Windows Settings › Privacy › Microphone.",
                )),
                Some("microphone"),
            ),
            Self::Other(message) => (tr("Aufnahme nicht möglich", "Recording failed"), Some(message.clone()), None),
        }
    }
}

fn classify(error: anyhow::Error) -> RecordingError {
    let text = format!("{error:#}");
    if text.contains("0x80070005") || text.to_lowercase().contains("access is denied") || text.contains("Zugriff verweigert") {
        RecordingError::MicrophoneDenied
    } else {
        RecordingError::Other(text)
    }
}

fn microphone_name(id: Option<&str>) -> Option<String> {
    let microphones = devices::microphones().ok()?;
    let wanted = id.map(str::to_string).or(microphones.default_id.clone())?;
    microphones.devices.into_iter().find(|d| d.id == wanted).map(|d| d.name)
}

/// Starts capturing the microphone into the sink. An unplugged device
/// switches to the system default.
fn start_microphone(app: &AppHandle, sink: Arc<Mutex<TrackSink>>, meters: Arc<Meters>, device: Option<String>) -> Result<Capture> {
    let source = CaptureSource::Microphone(device);
    let app = app.clone();
    let on_audio = move |samples: &[f32], timestamp: u64| {
        let mut sink = sink.lock().unwrap();
        match sink.append(samples, timestamp) {
            Ok(Some(progress)) => {
                if let Some(level) = progress.levels.last() {
                    Meters::set(&meters.microphone, &meters.microphone_at, *level);
                }
                meters.duration_ms.fetch_max((progress.duration * 1000.0) as u64, Ordering::Relaxed);
            }
            Ok(None) => {}
            Err(message) => tracing::error!(message, "writing microphone track failed"),
        }
    };
    let on_error = move |message: String| {
        tracing::warn!(message, "microphone lost");
        // Not on the capture thread: replacing the capture joins it.
        std::thread::spawn(move || microphone_lost(&app));
    };
    Capture::start(source, on_audio, on_error)
}

fn start_system(sink: Arc<Mutex<TrackSink>>, meters: Arc<Meters>, source: CaptureSource) -> Result<Capture> {
    let on_audio = move |samples: &[f32], timestamp: u64| {
        let mut sink = sink.lock().unwrap();
        match sink.append(samples, timestamp) {
            Ok(Some(progress)) => {
                if let Some(level) = progress.levels.last() {
                    Meters::set(&meters.system, &meters.system_at, *level);
                }
                meters.duration_ms.fetch_max((progress.duration * 1000.0) as u64, Ordering::Relaxed);
            }
            Ok(None) => {}
            Err(message) => tracing::error!(message, "writing system track failed"),
        }
    };
    Capture::start(source, on_audio, |message| tracing::error!(message, "system audio capture stopped"))
}

fn check_space(storage: &Storage) -> Result<(), RecordingError> {
    match storage.available_space() {
        Some(available) if available < MINIMUM_FREE_SPACE => Err(RecordingError::NotEnoughSpace),
        _ => Ok(()),
    }
}

/// Starts a microphone recording.
pub fn start_recording(app: &AppHandle) -> Result<(), RecordingError> {
    let state = app.state::<AppState>();
    let mut recorder = state.recorder.lock().unwrap();
    if recorder.is_active() {
        return Ok(());
    }
    let storage = state.current_storage();
    check_space(&storage)?;
    let settings = state.settings();
    let id = uuid::Uuid::new_v4().to_string();
    let manifest = RecordingManifest {
        id: id.clone(),
        title: format!("{} {}", tr("Aufnahme", "Recording"), format::short_date_time(Local::now())),
        created_at: chrono::Utc::now(),
        source: RecordingSource::Microphone,
        file_name: RECORDING_FILE.into(),
        microphone_file_name: None,
        system_file_name: None,
        folder_id: None,
    };
    let result = (|| -> Result<Active> {
        storage.make_folder(&id)?;
        storage.write_manifest(&manifest)?;
        let sink = Arc::new(Mutex::new(TrackSink::create(&storage.file(&id, RECORDING_FILE))?));
        sink.lock().unwrap().start_timeline(now_100ns());
        let meters = Arc::new(Meters::default());
        let device = (!settings.microphone.is_empty()).then(|| settings.microphone.clone());
        let microphone = match start_microphone(app, sink.clone(), meters.clone(), device.clone()) {
            Ok(capture) => capture,
            // The chosen microphone is gone: use the default.
            Err(_) if device.is_some() => start_microphone(app, sink.clone(), meters.clone(), None)?,
            Err(error) => return Err(error),
        };
        Ok(Active {
            manifest: manifest.clone(),
            storage: storage.clone(),
            paused: false,
            microphone_sink: sink,
            system_sink: None,
            microphone: Some(microphone),
            system: None,
            meters,
            source_name: None,
            microphone_name: microphone_name(device.as_deref()),
            ticker: Arc::new(AtomicBool::new(false)),
        })
    })();
    match result {
        Ok(active) => {
            start_ticker(app, active.ticker.clone());
            recorder.active = Some(active);
            tracing::info!(id = %id, "recording started");
            drop(recorder);
            emit_status(app);
            Ok(())
        }
        Err(error) => {
            let _ = storage.remove_folder(&id);
            Err(classify(error))
        }
    }
}

/// Records microphone and system audio as separate tracks. `app_exe`: the
/// app to record on its own (None = the setting, "" = all system audio).
pub fn start_meeting(app: &AppHandle, app_exe: Option<String>) -> Result<(), RecordingError> {
    let state = app.state::<AppState>();
    let mut recorder = state.recorder.lock().unwrap();
    if recorder.is_active() {
        return Ok(());
    }
    let storage = state.current_storage();
    check_space(&storage)?;
    let settings = state.settings();
    let exe = app_exe.unwrap_or(settings.meeting_source.clone());
    let target = (!exe.is_empty()).then(|| devices::find_app(&exe)).flatten();
    let date = format::short_date_time(Local::now());
    let id = uuid::Uuid::new_v4().to_string();
    let manifest = RecordingManifest {
        id: id.clone(),
        title: match &target {
            Some(app) => format!("Meeting {} {}", app.name, date),
            None => format!("Meeting {date}"),
        },
        created_at: chrono::Utc::now(),
        source: RecordingSource::Meeting,
        file_name: MIX_FILE.into(),
        microphone_file_name: Some(MICROPHONE_FILE.into()),
        system_file_name: Some(SYSTEM_FILE.into()),
        folder_id: None,
    };
    let mut fallback_notice = false;
    let result = (|| -> Result<Active> {
        storage.make_folder(&id)?;
        storage.write_manifest(&manifest)?;
        let microphone_sink = Arc::new(Mutex::new(TrackSink::create(&storage.file(&id, MICROPHONE_FILE))?));
        let system_sink = Arc::new(Mutex::new(TrackSink::create(&storage.file(&id, SYSTEM_FILE))?));
        let origin = now_100ns();
        microphone_sink.lock().unwrap().start_timeline(origin);
        system_sink.lock().unwrap().start_timeline(origin);
        let meters = Arc::new(Meters::default());

        let (system, source_name) = match &target {
            Some(target) => match start_system(system_sink.clone(), meters.clone(), CaptureSource::Application(target.process_id)) {
                Ok(capture) => (capture, Some(target.name.clone())),
                Err(error) => {
                    tracing::warn!(%error, "app audio unavailable, recording all system audio");
                    fallback_notice = true;
                    (start_system(system_sink.clone(), meters.clone(), CaptureSource::SystemAudio)?, None)
                }
            },
            None => (start_system(system_sink.clone(), meters.clone(), CaptureSource::SystemAudio)?, None),
        };
        let device = (!settings.microphone.is_empty()).then(|| settings.microphone.clone());
        let microphone = match start_microphone(app, microphone_sink.clone(), meters.clone(), device.clone()) {
            Ok(capture) => capture,
            Err(_) if device.is_some() => start_microphone(app, microphone_sink.clone(), meters.clone(), None)?,
            Err(error) => return Err(error),
        };
        Ok(Active {
            manifest: manifest.clone(),
            storage: storage.clone(),
            paused: false,
            microphone_sink,
            system_sink: Some(system_sink),
            microphone: Some(microphone),
            system: Some(system),
            meters,
            source_name,
            microphone_name: microphone_name(device.as_deref()),
            ticker: Arc::new(AtomicBool::new(false)),
        })
    })();
    match result {
        Ok(active) => {
            start_ticker(app, active.ticker.clone());
            recorder.active = Some(active);
            tracing::info!(id = %id, app = ?target.as_ref().map(|t| &t.exe), "meeting started");
            drop(recorder);
            if fallback_notice {
                state.notice(
                    tr("Gesamtes Systemaudio wird aufgenommen", "Recording all system audio"),
                    Some(tr(
                        "Die App ließ sich nicht einzeln aufnehmen. Scripta nimmt stattdessen alles auf, was der PC abspielt.",
                        "The app could not be recorded on its own. Scripta records everything the PC plays instead.",
                    )),
                    None,
                );
            }
            emit_status(app);
            Ok(())
        }
        Err(error) => {
            let _ = storage.remove_folder(&id);
            Err(classify(error))
        }
    }
}

/// Sends the meters to the UI ten times a second while recording.
fn start_ticker(app: &AppHandle, stop: Arc<AtomicBool>) {
    let app = app.clone();
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            emit_status(&app);
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

pub fn toggle_pause(app: &AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut recorder = state.recorder.lock().unwrap();
        let Some(active) = recorder.active.as_mut() else { return };
        let now = now_100ns();
        let sinks = std::iter::once(&active.microphone_sink).chain(active.system_sink.iter());
        for sink in sinks {
            let mut sink = sink.lock().unwrap();
            if active.paused {
                sink.resume_timeline(now);
            } else {
                sink.pause_timeline(now);
            }
        }
        active.paused = !active.paused;
    }
    emit_status(app);
}

/// Switches the microphone of a running recording.
pub fn switch_microphone(app: &AppHandle, device: Option<String>) -> Result<(), RecordingError> {
    let state = app.state::<AppState>();
    let mut recorder = state.recorder.lock().unwrap();
    let Some(active) = recorder.active.as_mut() else { return Ok(()) };
    if let Some(old) = active.microphone.take() {
        old.stop();
    }
    let capture = start_microphone(app, active.microphone_sink.clone(), active.meters.clone(), device.clone())
        .or_else(|_| start_microphone(app, active.microphone_sink.clone(), active.meters.clone(), None))
        .map_err(classify)?;
    active.microphone = Some(capture);
    active.microphone_name = microphone_name(device.as_deref());
    tracing::info!("microphone switched");
    Ok(())
}

/// The microphone disappeared (unplugged): continue with the system default,
/// or stop and save if there is none.
fn microphone_lost(app: &AppHandle) {
    let state = app.state::<AppState>();
    let restarted = {
        let mut recorder = state.recorder.lock().unwrap();
        let Some(active) = recorder.active.as_mut() else { return };
        if let Some(old) = active.microphone.take() {
            old.stop();
        }
        match start_microphone(app, active.microphone_sink.clone(), active.meters.clone(), None) {
            Ok(capture) => {
                active.microphone = Some(capture);
                active.microphone_name = microphone_name(None);
                true
            }
            Err(_) => false,
        }
    };
    if restarted {
        state.toast(tr("Mikrofon gewechselt", "Microphone switched"));
    } else {
        stop_recording(app);
        state.notice(
            tr("Kein Mikrofon mehr verfügbar", "No microphone available"),
            Some(tr("Die Aufnahme wurde beendet und gespeichert.", "The recording was stopped and saved.")),
            None,
        );
    }
}

/// Stops the recording and returns its files to finish.
fn stop_capture(app: &AppHandle) -> Option<Finished> {
    let state = app.state::<AppState>();
    let mut recorder = state.recorder.lock().unwrap();
    let mut active = recorder.active.take()?;
    active.ticker.store(true, Ordering::Relaxed);
    if let Some(capture) = active.microphone.take() {
        capture.stop();
    }
    if let Some(capture) = active.system.take() {
        capture.stop();
    }
    active.microphone_sink.lock().unwrap().finish();
    if let Some(sink) = &active.system_sink {
        sink.lock().unwrap().finish();
    }
    recorder.saving.push(active.manifest.source);
    tracing::info!(id = %active.manifest.id, "recording stopped");
    Some(Finished { manifest: active.manifest, storage: active.storage })
}

/// Stops recording and adds the recording to the library once its files are
/// finished in the background. Returns the thread doing that.
pub fn stop_recording(app: &AppHandle) -> Option<std::thread::JoinHandle<()>> {
    let finished = stop_capture(app)?;
    emit_status(app);
    let app = app.clone();
    Some(std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let source = finished.manifest.source;
        let levels = state.settings().levels_microphone;
        match finish(&finished.manifest, &finished.storage, levels) {
            Ok(duration) => {
                if let Err(error) = state.add_to_library(&finished.manifest, &finished.storage, duration) {
                    tracing::error!(%error, "adding recording failed");
                    state.notice(tr("Aufnahme nicht gespeichert", "Recording not saved"), Some(error.to_string()), None);
                } else {
                    let _ = app.emit("recording-added", &finished.manifest.id);
                }
            }
            Err(error) => {
                // The manifest stays on disk, so the recording is recovered on next launch.
                tracing::error!(%error, "finishing failed");
                state.notice(tr("Aufnahme nicht gespeichert", "Recording not saved"), Some(error.to_string()), None);
            }
        }
        {
            let mut recorder = state.recorder.lock().unwrap();
            if let Some(index) = recorder.saving.iter().position(|s| *s == source) {
                recorder.saving.remove(index);
            }
        }
        emit_status(&app);
    }))
}

/// Turns the captured tracks of a stopped recording into its final files:
/// levels the microphone and, for meetings, mixes both tracks into the
/// playback file. Returns the duration of the playback file.
pub fn finish(manifest: &RecordingManifest, storage: &Storage, levels_microphone: bool) -> Result<f64> {
    let output = storage.file(&manifest.id, &manifest.file_name);
    // Tracks that were already finished elsewhere (e.g. a meeting from Scripta
    // for the Mac, encoded and mixed): nothing to level or mix.
    if !manifest.file_name.ends_with(".wav") && output.exists() {
        return crate::audio::mf::duration(&output);
    }
    if levels_microphone {
        if let Some(name) = manifest.microphone_track().filter(|name| name.ends_with(".wav")) {
            if let Err(error) = leveler::level_file(&storage.file(&manifest.id, &name)) {
                // The unlevelled track is still fine to keep.
                tracing::error!(%error, "levelling failed");
            }
        }
    }
    if !manifest.is_meeting() {
        return wav::duration(&output).ok_or_else(|| anyhow!("the recording file is unreadable"));
    }
    let inputs: Vec<_> = manifest.captured_file_names().iter().map(|name| storage.file(&manifest.id, name)).collect();
    let paths: Vec<&std::path::Path> = inputs.iter().map(|p| p.as_path()).collect();
    mixer::mix(&paths, &output)
}
