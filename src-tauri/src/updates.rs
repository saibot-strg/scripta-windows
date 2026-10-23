//! Looks for a newer release on GitHub once a day and tells the UI. Sends
//! nothing but the request itself; no data about recordings or usage.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

const LATEST_RELEASE: &str = "https://api.github.com/repos/saibot-strg/scripta-windows/releases/latest";

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub page: String,
}

/// "0.10.1" > "0.9.3": numeric comparison of dot-separated parts.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v').split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Asks GitHub now. Returns the newer release, if any.
pub fn check_now(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("Scripta-Windows/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(LATEST_RELEASE).header("Accept", "application/vnd.github+json").send().map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    state.modify_settings(|s| s.last_update_check = Some(Utc::now().to_rfc3339()));
    if !response.status().is_success() {
        return Err(format!("GitHub: {}", response.status()));
    }
    let release: serde_json::Value = serde_json::from_str(&response.text().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let version = release["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_string();
    let page = release["html_url"].as_str().unwrap_or_default().to_string();
    if version.is_empty() {
        return Ok(None);
    }
    state.modify_settings(|s| {
        s.latest_release_version = Some(version.clone());
        s.latest_release_page = Some(page.clone());
    });
    Ok(is_newer(&version, current_version()).then_some(UpdateInfo { version, page }))
}

/// The newer release known from the last check that the user didn't skip.
pub fn pending(app: &AppHandle) -> Option<UpdateInfo> {
    let settings = app.state::<AppState>().settings();
    let version = settings.latest_release_version?;
    if !is_newer(&version, current_version()) || settings.skipped_update_version.as_deref() == Some(version.as_str()) {
        return None;
    }
    Some(UpdateInfo { version, page: settings.latest_release_page.unwrap_or_default() })
}

/// Checks once a day while the app runs (if the setting is on).
pub fn start_automatic(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        let settings = app.state::<AppState>().settings();
        let due = settings
            .last_update_check
            .as_deref()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .is_none_or(|last| Utc::now() - last > chrono::Duration::hours(24));
        if settings.checks_for_updates && due {
            match check_now(&app) {
                Ok(_) => {
                    if let Some(update) = pending(&app) {
                        let _ = app.emit("update-available", update);
                    }
                }
                Err(error) => tracing::info!(error, "update check failed"),
            }
        }
        std::thread::sleep(Duration::from_secs(6 * 3600));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }
}
