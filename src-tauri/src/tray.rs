//! The icon in the notification area: open Scripta, start or stop a
//! recording, quit.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::recorder;
use crate::state::{tr, AppState};

const TRAY_ID: &str = "main";

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Quits, or asks first while a recording is running.
pub fn request_quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let busy = {
        let recorder = state.recorder.lock().unwrap();
        recorder.is_active() || !recorder.saving.is_empty()
    };
    if busy {
        show_main_window(app);
        let _ = app.emit("confirm-quit", ());
    } else {
        app.exit(0);
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", tr("Scripta öffnen", "Open Scripta"), true, None::<&str>)?;
    let record = MenuItem::with_id(app, "record", tr("Neue Aufnahme", "New Recording"), true, None::<&str>)?;
    let meeting = MenuItem::with_id(app, "meeting", tr("Meeting aufnehmen", "Record Meeting"), true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", tr("Aufnahme beenden", "Stop Recording"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", tr("Scripta beenden", "Quit Scripta"), true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &record, &meeting, &stop, &separator2, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Scripta")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "record" => {
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Err(error) = recorder::start_recording(&app) {
                        let (title, message, action) = error.notice();
                        show_main_window(&app);
                        app.state::<AppState>().notice(title, message, action);
                    }
                });
            }
            "meeting" => {
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Err(error) = recorder::start_meeting(&app, None) {
                        let (title, message, action) = error.notice();
                        show_main_window(&app);
                        app.state::<AppState>().notice(title, message, action);
                    }
                });
            }
            "stop" => {
                recorder::stop_recording(app);
            }
            "quit" => request_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;
    let visible = app.state::<AppState>().settings().shows_tray_icon;
    let _ = tray.set_visible(visible);
    Ok(())
}

pub fn set_visible(app: &AppHandle, visible: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(visible);
    }
}
