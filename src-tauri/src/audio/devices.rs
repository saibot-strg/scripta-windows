//! Microphones and apps that can be recorded in a meeting.

use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;
use wasapi::{DeviceEnumerator, Direction};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Microphone {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Microphones {
    pub devices: Vec<Microphone>,
    pub default_id: Option<String>,
}

/// Active capture endpoints and the default one.
pub fn microphones() -> Result<Microphones> {
    let _ = wasapi::initialize_mta();
    let enumerator = DeviceEnumerator::new()?;
    let collection = enumerator.get_device_collection(&Direction::Capture)?;
    let mut devices = Vec::new();
    for device in &collection {
        let device = device?;
        devices.push(Microphone { id: device.get_id()?, name: device.get_friendlyname()? });
    }
    let default_id = enumerator.get_default_device(&Direction::Capture).ok().and_then(|d| d.get_id().ok());
    Ok(Microphones { devices, default_id })
}

/// An app whose audio can be recorded on its own.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingApp {
    /// Executable name, lowercase ("ms-teams.exe"); stored in the settings.
    pub exe: String,
    pub name: String,
    #[serde(skip)]
    pub process_id: u32,
}

/// Meeting and browser apps, recognized even before they play sound.
const KNOWN_APPS: &[(&str, &str)] = &[
    ("ms-teams.exe", "Microsoft Teams"),
    ("teams.exe", "Microsoft Teams (classic)"),
    ("zoom.exe", "Zoom"),
    ("webex.exe", "Webex"),
    ("ciscocollabhost.exe", "Webex"),
    ("webexmta.exe", "Webex"),
    ("discord.exe", "Discord"),
    ("slack.exe", "Slack"),
    ("skype.exe", "Skype"),
    ("lync.exe", "Skype for Business"),
    ("g2mcomm.exe", "GoTo Meeting"),
    ("gotomeeting.exe", "GoTo Meeting"),
    ("signal.exe", "Signal"),
    ("whatsapp.exe", "WhatsApp"),
    ("chrome.exe", "Google Chrome"),
    ("msedge.exe", "Microsoft Edge"),
    ("firefox.exe", "Firefox"),
    ("brave.exe", "Brave"),
    ("opera.exe", "Opera"),
    ("vivaldi.exe", "Vivaldi"),
];

/// Processes that start apps; an app's top process is the one below them.
const LAUNCHERS: &[&str] = &[
    "explorer.exe",
    "svchost.exe",
    "services.exe",
    "wininit.exe",
    "winlogon.exe",
    "userinit.exe",
    "sihost.exe",
    "runtimebroker.exe",
    "smss.exe",
    "csrss.exe",
    "system",
];

struct ProcessInfo {
    parent: u32,
    exe: String,
}

fn processes() -> HashMap<u32, ProcessInfo> {
    let mut table = HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return table };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let length = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
                let exe = String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase();
                table.insert(entry.th32ProcessID, ProcessInfo { parent: entry.th32ParentProcessID, exe });
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    table
}

/// The top process of the app `pid` belongs to (e.g. ms-teams.exe for one
/// of its WebView2 helpers).
fn top_process(pid: u32, table: &HashMap<u32, ProcessInfo>) -> u32 {
    let mut current = pid;
    for _ in 0..32 {
        let Some(info) = table.get(&current) else { break };
        let Some(parent) = table.get(&info.parent) else { break };
        if info.parent == current || info.parent == 0 || LAUNCHERS.contains(&parent.exe.as_str()) {
            break;
        }
        current = info.parent;
    }
    current
}

fn display_name(exe: &str) -> String {
    if let Some((_, name)) = KNOWN_APPS.iter().find(|(known, _)| *known == exe) {
        return name.to_string();
    }
    let stem = exe.trim_end_matches(".exe");
    let mut chars = stem.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => exe.to_string(),
    }
}

/// Apps playing sound right now plus known meeting apps that are running.
pub fn meeting_apps() -> Vec<MeetingApp> {
    let table = processes();
    let mut apps: Vec<MeetingApp> = Vec::new();
    let add = |pid: u32, apps: &mut Vec<MeetingApp>| {
        let top = top_process(pid, &table);
        let Some(info) = table.get(&top) else { return };
        if LAUNCHERS.contains(&info.exe.as_str()) || info.exe == "audiodg.exe" || info.exe == "scripta.exe" {
            return;
        }
        if !apps.iter().any(|a| a.exe == info.exe) {
            apps.push(MeetingApp { exe: info.exe.clone(), name: display_name(&info.exe), process_id: top });
        }
    };
    for pid in sessions_with_audio() {
        add(pid, &mut apps);
    }
    for (pid, info) in &table {
        if KNOWN_APPS.iter().any(|(exe, _)| *exe == info.exe) {
            add(*pid, &mut apps);
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// The running app with this executable, if any.
pub fn find_app(exe: &str) -> Option<MeetingApp> {
    let table = processes();
    let pid = table.iter().find(|(_, info)| info.exe == exe).map(|(pid, _)| *pid)?;
    let top = top_process(pid, &table);
    let info = table.get(&top)?;
    // The top of a tree of the same executable (browsers run many).
    let top = if info.exe == exe { top } else { pid };
    Some(MeetingApp { exe: exe.to_string(), name: display_name(exe), process_id: top })
}

/// Process IDs with an audio session on the default output.
fn sessions_with_audio() -> Vec<u32> {
    let _ = wasapi::initialize_mta();
    let result = (|| -> Result<Vec<u32>> {
        let enumerator = DeviceEnumerator::new()?;
        let device = enumerator.get_default_device(&Direction::Render)?;
        let manager = device.get_iaudiosessionmanager()?;
        let sessions = manager.get_audiosessionenumerator()?;
        let mut pids = Vec::new();
        for index in 0..sessions.get_count()? {
            let session = sessions.get_session(index)?;
            if let Ok(pid) = session.get_process_id() {
                if pid != 0 {
                    pids.push(pid);
                }
            }
        }
        Ok(pids)
    })();
    result.unwrap_or_default()
}
