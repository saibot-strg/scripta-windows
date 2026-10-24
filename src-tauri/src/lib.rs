//! Scripta for Windows: local transcription for meetings and voice notes.

pub mod audio;
pub mod commands;
pub mod engine;
pub mod export;
pub mod format;
pub mod gpu;
pub mod importer;
pub mod library;
pub mod logging;
pub mod model;
pub mod models;
pub mod paths;
pub mod priority;
pub mod recorder;
pub mod recovery;
pub mod service;
pub mod settings;
pub mod speakers;
pub mod state;
pub mod storage;
pub mod transcript;
pub mod tray;
pub mod updates;
pub mod voices;

use tauri::window::{Effect, EffectsBuilder};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::library::Library;
use crate::model::Language;
use crate::models::ModelKind;
use crate::settings::SettingsStore;
use crate::state::{tr, AppState};
use crate::voices::VoiceStore;

/// Windows 11 draws a Mica backdrop behind the transparent page.
pub fn uses_mica() -> bool {
    windows_version::OsVersion::current().build >= 22_000
}

fn system_language() -> Language {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    // Primary language ID 0x07 = German.
    if unsafe { GetUserDefaultUILanguage() } & 0x3ff == 0x07 {
        Language::German
    } else {
        Language::English
    }
}

fn create_window(app: &tauri::App) -> tauri::Result<()> {
    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Scripta")
        .inner_size(1180.0, 760.0)
        .min_inner_size(880.0, 560.0)
        .center()
        // Shown by the page once it has rendered (no white flash).
        .visible(false);
    if uses_mica() {
        builder = builder.transparent(true).effects(EffectsBuilder::new().effect(Effect::Mica).build());
    }
    builder.build()?;
    Ok(())
}

/// Opens the databases; if one can't be opened, the app still starts with
/// an empty one in memory and says so.
fn open_stores() -> (Library, VoiceStore, Option<String>) {
    let _ = std::fs::create_dir_all(paths::data_dir());
    let mut problem = None;
    let library = Library::open(&paths::library_file()).unwrap_or_else(|error| {
        tracing::error!(%error, "library unavailable");
        problem = Some(error.to_string());
        Library::open_in_memory().expect("in-memory library")
    });
    let voices = VoiceStore::open(&paths::voices_file()).unwrap_or_else(|error| {
        tracing::error!(%error, "voice store unavailable");
        problem = Some(error.to_string());
        VoiceStore::open_in_memory().expect("in-memory voice store")
    });
    (library, voices, problem)
}

/// Recovers interrupted recordings, resumes queued jobs and loads the
/// models in the background.
fn startup(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    recovery::run(app);
    if gpu::Guard::take_crash() && state.settings().uses_gpu {
        tracing::warn!("the last run ended during work on the graphics card; back to the processor");
        state.modify_settings(|s| {
            s.uses_gpu = false;
            s.gpu_failed = true;
        });
    }
    let pending = state.library.lock().unwrap().pending_jobs().unwrap_or_default();
    for id in pending {
        state.service.enqueue(&id, None);
    }
    updates::start_automatic(app);
    let settings = state.settings();
    // First launch: the download starts once the welcome screen was read.
    if settings.welcome_done {
        if !ModelKind::Speech.is_installed() {
            state.service.prepare(ModelKind::Speech);
        }
        if settings.separates_speakers && !ModelKind::Speakers.is_installed() {
            state.service.prepare(ModelKind::Speakers);
        }
    }
}

pub fn run() {
    let _log = logging::start();
    Language::set_current(system_language());
    tracing::info!(version = env!("CARGO_PKG_VERSION"), mica = uses_mica(), "Scripta starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // Setup runs on the window's thread.
            priority::raise_current_thread();
            let (library, voices, problem) = open_stores();
            let settings = SettingsStore::load(&paths::settings_file());
            app.manage(AppState::new(app.handle().clone(), library, voices, settings));
            let state = app.state::<AppState>();
            state.allow_audio_folder(&paths::default_recordings_root());
            state.allow_audio_folder(&state.current_storage().root);
            create_window(app)?;
            tray::setup(app.handle())?;
            if let Some(problem) = problem {
                state.notice(
                    tr("Die Bibliothek konnte nicht geöffnet werden", "The library could not be opened"),
                    Some(problem),
                    None,
                );
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || startup(&handle));
            // The page shows the window once it has rendered; if it never
            // does (a broken page), show it anyway.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(4));
                if let Some(window) = handle.get_webview_window("main") {
                    if !window.is_visible().unwrap_or(true) {
                        let _ = window.show();
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let state = app.state::<AppState>();
                let busy = {
                    let recorder = state.recorder.lock().unwrap();
                    recorder.is_active() || !recorder.saving.is_empty()
                };
                if busy {
                    api.prevent_close();
                    let _ = app.emit("confirm-quit", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::list_recordings,
            commands::list_folders,
            commands::search,
            commands::get_recording,
            commands::rename_recording,
            commands::delete_recording,
            commands::move_to_folder,
            commands::create_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::recording_file,
            commands::transcribe,
            commands::cancel_transcription,
            commands::transcribe_all_pending,
            commands::service_snapshot,
            commands::prepare_model,
            commands::delete_model,
            commands::model_sizes,
            commands::graphics_cards,
            commands::edit_segment,
            commands::restore_segment,
            commands::rename_speaker,
            commands::assign_names,
            commands::confirm_suggestion,
            commands::merge_speaker,
            commands::move_segment,
            commands::move_segment_to_new_speaker,
            commands::list_people,
            commands::has_voice_data,
            commands::rename_person,
            commands::delete_person,
            commands::delete_all_voices,
            commands::export_transcript,
            commands::transcript_text,
            commands::export_file_name,
            commands::import_files,
            commands::import_extensions,
            commands::start_recording,
            commands::start_meeting,
            commands::stop_recording,
            commands::toggle_pause,
            commands::recorder_status,
            commands::list_microphones,
            commands::select_microphone,
            commands::list_meeting_apps,
            commands::quit_after_saving,
            commands::get_settings,
            commands::update_settings,
            commands::set_storage_root,
            commands::current_storage_root,
            commands::open_privacy_settings,
            commands::open_sound_settings,
            commands::open_storage_folder,
            commands::open_logs_folder,
            commands::check_updates,
            commands::skip_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Scripta")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                gpu::Guard::clear();
            }
        });
}
