//! Commands the UI calls. Errors come back as text for the alert.
//!
//! Tauri runs synchronous commands on the window's main thread, so one that
//! reads the library or waits for a lock freezes the window meanwhile; all of
//! them are marked `async` to run on Tauri's thread pool instead.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::audio::devices::{self, MeetingApp, Microphones};
use crate::export::{self, ExportFormat};
use crate::format;
use crate::importer;
use crate::library::{FolderInfo, RecordingSummary, TranscriptView};
use crate::model::{Language, TrackRole, TranscriptionStatus};
use crate::models::ModelKind;
use crate::recorder::{self, RecorderStatus};
use crate::service::ServiceSnapshot;
use crate::settings::Settings;
use crate::speakers::{NameAssignment, Speakers};
use crate::state::{AppState, Notice};
use crate::storage::Storage;
use crate::updates::{self, UpdateInfo};
use crate::voices::{PersonSummary, VoiceStatus};

type Reply<T> = Result<T, String>;

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

// MARK: - Startup

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub settings: Settings,
    pub version: String,
    /// The window has a Mica backdrop (Windows 11): keep the page transparent.
    pub mica: bool,
    pub recordings_root: String,
    pub is_default_storage: bool,
    pub update: Option<UpdateInfo>,
    pub speech_download_size: u64,
    pub speakers_download_size: u64,
}

#[tauri::command(async)]
pub fn bootstrap(app: AppHandle, state: State<AppState>, language: String) -> Bootstrap {
    Language::set_current(if language.starts_with("de") { Language::German } else { Language::English });
    let settings = state.settings();
    let storage = state.current_storage();
    Bootstrap {
        is_default_storage: settings.storage_root.is_none(),
        settings,
        version: updates::current_version().to_string(),
        mica: crate::uses_mica(),
        recordings_root: storage.root.to_string_lossy().into_owned(),
        update: updates::pending(&app),
        speech_download_size: ModelKind::Speech.size(),
        speakers_download_size: ModelKind::Speakers.size(),
    }
}

// MARK: - Library

#[tauri::command(async)]
pub fn list_recordings(state: State<AppState>) -> Reply<Vec<RecordingSummary>> {
    let library = state.library.lock().unwrap();
    let mut summaries = library.summaries().map_err(text)?;
    let files: HashMap<String, PathBuf> = library
        .playback_files()
        .map_err(text)?
        .into_iter()
        .map(|(id, root, file)| {
            let path = Storage::for_root(root.as_deref()).file(&id, &file);
            (id, path)
        })
        .collect();
    for summary in &mut summaries {
        summary.has_audio = files.get(&summary.id).is_some_and(|p| p.exists());
    }
    Ok(summaries)
}

#[tauri::command(async)]
pub fn list_folders(state: State<AppState>) -> Reply<Vec<FolderInfo>> {
    state.library.lock().unwrap().folders().map_err(text)
}

#[tauri::command(async)]
pub fn search(state: State<AppState>, query: String) -> Reply<Vec<String>> {
    state.library.lock().unwrap().search(&query).map_err(text)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDetail {
    pub recording: RecordingSummary,
    /// The playback file, if it exists.
    pub audio_path: Option<String>,
    pub transcript: Option<TranscriptView>,
    /// Voice status by speaker ID.
    pub voices: HashMap<String, VoiceStatus>,
    /// Seconds spoken by speaker ID.
    pub speech: HashMap<String, f64>,
}

fn playback_path(state: &AppState, id: &str) -> Option<PathBuf> {
    let library = state.library.lock().unwrap();
    let recording = library.recording(id).ok()??;
    let track = library.tracks(id).ok()?.into_iter().find(|t| t.role == TrackRole::Primary)?;
    let path = Storage::for_root(recording.storage_root.as_deref()).file(id, &track.file_name);
    path.exists().then_some(path)
}

#[tauri::command(async)]
pub fn get_recording(state: State<AppState>, id: String) -> Reply<Option<RecordingDetail>> {
    let audio_path = playback_path(&state, &id);
    if let Some(path) = &audio_path {
        if let Some(folder) = path.parent() {
            state.allow_audio_folder(folder);
        }
    }
    let library = state.library.lock().unwrap();
    let Some(mut recording) = library.summary(&id).map_err(text)? else { return Ok(None) };
    recording.has_audio = audio_path.is_some();
    let transcript = library.transcript_view(&id).map_err(text)?;
    let (voices, speech) = match &transcript {
        Some(transcript) => {
            let ids: Vec<String> = transcript.speakers.iter().map(|s| s.id.clone()).collect();
            (state.voices.lock().unwrap().statuses(&ids).map_err(text)?, library.speech_seconds(&transcript.id).map_err(text)?)
        }
        None => (HashMap::new(), HashMap::new()),
    };
    Ok(Some(RecordingDetail {
        recording,
        audio_path: audio_path.map(|p| p.to_string_lossy().into_owned()),
        transcript,
        voices,
        speech,
    }))
}

#[tauri::command(async)]
pub fn rename_recording(state: State<AppState>, id: String, title: String) -> Reply<()> {
    state.library.lock().unwrap().rename(&id, &title).map_err(text)?;
    state.notify_library(Some(&id));
    Ok(())
}

#[tauri::command(async)]
pub fn delete_recording(state: State<AppState>, id: String) -> Reply<()> {
    state.cancel(&id);
    let root = {
        let library = state.library.lock().unwrap();
        library.recording(&id).map_err(text)?.map(|r| r.storage_root)
    };
    let Some(root) = root else { return Ok(()) };
    state.voices.lock().unwrap().forget_recording(&id).map_err(text)?;
    state.library.lock().unwrap().delete(&id).map_err(text)?;
    if let Err(error) = Storage::for_root(root.as_deref()).remove_folder(&id) {
        tracing::warn!(%error, "recording folder not removed");
    }
    tracing::info!(id, "recording deleted");
    state.notify_library(None);
    Ok(())
}

#[tauri::command(async)]
pub fn move_to_folder(state: State<AppState>, ids: Vec<String>, folder_id: Option<String>) -> Reply<()> {
    state.library.lock().unwrap().move_to_folder(&ids, folder_id.as_deref()).map_err(text)?;
    state.notify_library(None);
    Ok(())
}

#[tauri::command(async)]
pub fn create_folder(state: State<AppState>, name: String) -> Reply<FolderInfo> {
    let folder = state.library.lock().unwrap().create_folder(&name).map_err(text)?;
    state.notify_library(None);
    Ok(folder)
}

#[tauri::command(async)]
pub fn rename_folder(state: State<AppState>, id: String, name: String) -> Reply<()> {
    state.library.lock().unwrap().rename_folder(&id, &name).map_err(text)?;
    state.notify_library(None);
    Ok(())
}

#[tauri::command(async)]
pub fn delete_folder(state: State<AppState>, id: String) -> Reply<()> {
    state.library.lock().unwrap().delete_folder(&id).map_err(text)?;
    state.notify_library(None);
    Ok(())
}

#[tauri::command(async)]
pub fn recording_file(state: State<AppState>, id: String) -> Option<String> {
    playback_path(&state, &id).map(|p| p.to_string_lossy().into_owned())
}

// MARK: - Transcription

#[tauri::command(async)]
pub fn transcribe(state: State<AppState>, id: String, speaker_count: Option<usize>) {
    state.enqueue(&id, speaker_count);
}

#[tauri::command(async)]
pub fn cancel_transcription(state: State<AppState>, id: String) {
    state.cancel(&id);
}

/// Queues every recording without a transcript (batch transcription).
#[tauri::command(async)]
pub fn transcribe_all_pending(state: State<AppState>) -> Reply<usize> {
    let summaries = list_recordings(state.clone())?;
    let mut count = 0;
    for summary in summaries.iter().rev() {
        if summary.has_audio && matches!(summary.status, TranscriptionStatus::NotTranscribed | TranscriptionStatus::Failed) {
            state.enqueue(&summary.id, None);
            count += 1;
        }
    }
    Ok(count)
}

#[tauri::command(async)]
pub fn service_snapshot(state: State<AppState>) -> ServiceSnapshot {
    state.service.snapshot()
}

fn model_kind(kind: &str) -> ModelKind {
    if kind == "speakers" {
        ModelKind::Speakers
    } else {
        ModelKind::Speech
    }
}

#[tauri::command(async)]
pub fn prepare_model(state: State<AppState>, kind: String) {
    state.service.prepare(model_kind(&kind));
}

#[tauri::command(async)]
pub fn delete_model(state: State<AppState>, kind: String) {
    state.service.delete_model(model_kind(&kind));
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSizes {
    pub speech: Option<u64>,
    pub speakers: Option<u64>,
    /// Download size of the speech model for the chosen device.
    pub speech_download: u64,
}

#[tauri::command(async)]
pub fn model_sizes(state: State<AppState>) -> ModelSizes {
    let settings = state.settings();
    let speech = ModelKind::Speech.on(crate::gpu::Device::choose(settings.uses_gpu, &settings.gpu_name));
    ModelSizes { speech: speech.installed_size(), speakers: ModelKind::Speakers.installed_size(), speech_download: speech.size() }
}

/// The graphics cards transcription can run on.
#[tauri::command(async)]
pub fn graphics_cards() -> Vec<crate::gpu::GraphicsCard> {
    crate::gpu::cards()
}

// MARK: - Editing

#[tauri::command(async)]
pub fn edit_segment(state: State<AppState>, id: String, text_value: String) -> Reply<()> {
    state.library.lock().unwrap().edit_segment(&id, &text_value).map_err(text)?;
    state.notify_library(None);
    Ok(())
}

#[tauri::command(async)]
pub fn restore_segment(state: State<AppState>, id: String) -> Reply<()> {
    state.library.lock().unwrap().restore_segment(&id).map_err(text)?;
    state.notify_library(None);
    Ok(())
}

fn with_speakers<T>(state: &AppState, action: impl FnOnce(&mut Speakers) -> anyhow::Result<T>) -> Reply<T> {
    let recognizes = state.settings().recognizes_voices;
    let result = {
        let mut library = state.library.lock().unwrap();
        let mut voices = state.voices.lock().unwrap();
        let mut speakers = Speakers { library: &mut library, voices: &mut voices, recognizes };
        action(&mut speakers).map_err(text)
    };
    state.notify_library(None);
    let _ = tauri::Emitter::emit(&state.app, "voices-changed", ());
    result
}

#[tauri::command(async)]
pub fn rename_speaker(state: State<AppState>, id: String, name: String, remember: bool) -> Reply<()> {
    with_speakers(&state, |s| s.rename(&id, &name, remember))
}

#[tauri::command(async)]
pub fn assign_names(state: State<AppState>, transcript_id: String, assignments: Vec<NameAssignment>) -> Reply<()> {
    with_speakers(&state, |s| s.assign_names(&transcript_id, &assignments))
}

#[tauri::command(async)]
pub fn confirm_suggestion(state: State<AppState>, id: String) -> Reply<()> {
    with_speakers(&state, |s| s.confirm_suggestion(&id))
}

#[tauri::command(async)]
pub fn merge_speaker(state: State<AppState>, id: String, target_id: String) -> Reply<()> {
    with_speakers(&state, |s| s.merge(&id, &target_id))
}

#[tauri::command(async)]
pub fn move_segment(state: State<AppState>, segment_id: String, speaker_id: String) -> Reply<()> {
    with_speakers(&state, |s| s.move_segment(&segment_id, &speaker_id))
}

#[tauri::command(async)]
pub fn move_segment_to_new_speaker(state: State<AppState>, segment_id: String, name: Option<String>) -> Reply<()> {
    with_speakers(&state, |s| s.move_to_new_speaker(&segment_id, name.as_deref()))
}

// MARK: - People

#[tauri::command(async)]
pub fn list_people(state: State<AppState>) -> Reply<Vec<PersonSummary>> {
    state.voices.lock().unwrap().people().map_err(text)
}

#[tauri::command(async)]
pub fn has_voice_data(state: State<AppState>) -> bool {
    state.voices.lock().unwrap().has_data().unwrap_or(false)
}

#[tauri::command(async)]
pub fn rename_person(state: State<AppState>, id: String, name: String) -> Reply<()> {
    with_speakers(&state, |s| s.rename_person(&id, &name))
}

#[tauri::command(async)]
pub fn delete_person(state: State<AppState>, id: String) -> Reply<()> {
    with_speakers(&state, |s| s.voices.delete_person(&id))
}

#[tauri::command(async)]
pub fn delete_all_voices(state: State<AppState>) -> Reply<()> {
    with_speakers(&state, |s| s.voices.delete_all())
}

// MARK: - Export

#[tauri::command(async)]
pub fn export_transcript(state: State<AppState>, id: String, format: ExportFormat, path: String) -> Reply<()> {
    let document = state.library.lock().unwrap().export_document(&id).map_err(text)?.ok_or("no transcript")?;
    std::fs::write(&path, export::render(&document, format)).map_err(text)?;
    Ok(())
}

#[tauri::command(async)]
pub fn transcript_text(state: State<AppState>, id: String, include_header: bool) -> Reply<String> {
    let document = state.library.lock().unwrap().export_document(&id).map_err(text)?.ok_or("no transcript")?;
    Ok(export::plain_text(&document, include_header))
}

/// A file name for the export dialog.
#[tauri::command(async)]
pub fn export_file_name(state: State<AppState>, id: String) -> String {
    let title = state.library.lock().unwrap().recording(&id).ok().flatten().map(|r| r.title).unwrap_or_default();
    format::file_name(&title)
}

// MARK: - Import

#[tauri::command(async)]
pub fn import_files(app: AppHandle, paths: Vec<String>, folder_id: Option<String>) {
    if !paths.is_empty() {
        importer::import(&app, paths, folder_id);
    }
}

#[tauri::command(async)]
pub fn import_extensions() -> Vec<String> {
    importer::EXTENSIONS.iter().map(|e| e.to_string()).collect()
}

// MARK: - Recording

fn notice_of(error: recorder::RecordingError) -> Notice {
    let (title, message, action) = error.notice();
    Notice { title, message, action: action.map(str::to_string) }
}

#[tauri::command]
pub async fn start_recording(app: AppHandle) -> Result<(), Notice> {
    tauri::async_runtime::spawn_blocking(move || recorder::start_recording(&app).map_err(notice_of))
        .await
        .unwrap_or_else(|e| Err(Notice { title: e.to_string(), message: None, action: None }))
}

#[tauri::command]
pub async fn start_meeting(app: AppHandle, app_exe: Option<String>) -> Result<(), Notice> {
    tauri::async_runtime::spawn_blocking(move || recorder::start_meeting(&app, app_exe).map_err(notice_of))
        .await
        .unwrap_or_else(|e| Err(Notice { title: e.to_string(), message: None, action: None }))
}

#[tauri::command(async)]
pub fn stop_recording(app: AppHandle) {
    recorder::stop_recording(&app);
}

#[tauri::command(async)]
pub fn toggle_pause(app: AppHandle) {
    recorder::toggle_pause(&app);
}

#[tauri::command(async)]
pub fn recorder_status(state: State<AppState>) -> RecorderStatus {
    state.recorder.lock().unwrap().status()
}

#[tauri::command]
pub async fn list_microphones() -> Reply<Microphones> {
    tauri::async_runtime::spawn_blocking(devices::microphones).await.map_err(text)?.map_err(text)
}

/// Chooses the microphone ("" = system default); a running recording
/// switches over right away.
#[tauri::command]
pub async fn select_microphone(app: AppHandle, id: String) -> Result<(), Notice> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.modify_settings(|s| s.microphone = id.clone());
        if state.recorder.lock().unwrap().is_active() {
            recorder::switch_microphone(&app, (!id.is_empty()).then_some(id)).map_err(notice_of)?;
        }
        recorder::emit_status(&app);
        Ok(())
    })
    .await
    .unwrap_or_else(|e| Err(Notice { title: e.to_string(), message: None, action: None }))
}

#[tauri::command]
pub async fn list_meeting_apps() -> Vec<MeetingApp> {
    tauri::async_runtime::spawn_blocking(devices::meeting_apps).await.unwrap_or_default()
}

/// Stops a running recording, waits until all recordings are saved, then quits.
#[tauri::command(async)]
pub fn quit_after_saving(app: AppHandle) {
    std::thread::spawn(move || {
        if let Some(saving) = recorder::stop_recording(&app) {
            let _ = saving.join();
        }
        while !app.state::<AppState>().recorder.lock().unwrap().saving.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        app.exit(0);
    });
}

// MARK: - Settings

#[tauri::command(async)]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings()
}

#[tauri::command(async)]
pub fn update_settings(app: AppHandle, state: State<AppState>, patch: serde_json::Value) -> Reply<Settings> {
    let settings = state.update_settings(&patch).map_err(text)?;
    if patch.get("showsTrayIcon").is_some() {
        crate::tray::set_visible(&app, settings.shows_tray_icon);
    }
    if patch.get("separatesSpeakers").and_then(|v| v.as_bool()) == Some(true) && !ModelKind::Speakers.is_installed() {
        state.service.prepare(ModelKind::Speakers);
    }
    // The graphics card needs its own speech model: download and load it now.
    if (patch.get("usesGpu").is_some() || patch.get("gpuName").is_some()) && settings.welcome_done {
        state.service.prepare(ModelKind::Speech);
    }
    Ok(settings)
}

/// Chooses the folder for new recordings (None = the default).
#[tauri::command(async)]
pub fn set_storage_root(state: State<AppState>, path: Option<String>) -> Reply<String> {
    if state.recorder.lock().unwrap().is_active() {
        return Err("recording".into());
    }
    if let Some(path) = &path {
        std::fs::create_dir_all(path).map_err(text)?;
        state.allow_audio_folder(std::path::Path::new(path));
    }
    state.modify_settings(|s| s.storage_root = path.clone());
    Ok(state.current_storage().root.to_string_lossy().into_owned())
}

#[tauri::command(async)]
pub fn current_storage_root(state: State<AppState>) -> String {
    let root = state.current_storage().root;
    let _ = std::fs::create_dir_all(&root);
    root.to_string_lossy().into_owned()
}

fn open_with_shell(target: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(target).spawn();
}

#[tauri::command(async)]
pub fn open_privacy_settings() {
    open_with_shell("ms-settings:privacy-microphone");
}

#[tauri::command(async)]
pub fn open_sound_settings() {
    open_with_shell("ms-settings:sound");
}

#[tauri::command(async)]
pub fn open_storage_folder(state: State<AppState>) {
    let root = state.current_storage().root;
    let _ = std::fs::create_dir_all(&root);
    open_with_shell(&root.to_string_lossy());
}

#[tauri::command(async)]
pub fn open_logs_folder() {
    let dir = crate::paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    open_with_shell(&dir.to_string_lossy());
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || updates::check_now(&app)).await.map_err(text)?
}

#[tauri::command(async)]
pub fn skip_update(state: State<AppState>, version: String) {
    state.modify_settings(|s| s.skipped_update_version = Some(version));
}
