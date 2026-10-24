//! The graphics card for transcription. ONNX Runtime's DirectML runs on any
//! DirectX 12 graphics card (AMD, NVIDIA and Intel alike).

use std::path::PathBuf;

use serde::Serialize;

use crate::paths;

/// Where the models run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Cpu,
    /// A graphics card through DirectML, by its LUID: adapter numbers
    /// differ between sessions and APIs (over SSH the RX 7900 XTX of the test
    /// PC was DXGI adapter 1, on the desktop 0, and onnxruntime's adapter 0
    /// was then the integrated graphics).
    Gpu(u64),
}

impl Device {
    /// The graphics card named `card` ("" or not found: the one with the most
    /// dedicated memory, i.e. the discrete one on PCs that also have an
    /// integrated one) if `gpu` is on and there is one, else the processor.
    pub fn choose(gpu: bool, card: &str) -> Device {
        if !gpu {
            return Device::Cpu;
        }
        let cards = cards();
        let chosen = cards.iter().find(|c| !card.is_empty() && c.name == card).or_else(|| cards.iter().max_by_key(|c| c.memory));
        chosen.map_or(Device::Cpu, |c| Device::Gpu(c.luid))
    }

    /// For tests and development tools: SCRIPTA_GPU=1 picks the graphics card
    /// with the most memory, SCRIPTA_GPU=<name> a card by name.
    pub fn from_env() -> Device {
        let gpu = std::env::var("SCRIPTA_GPU").ok();
        Device::choose(gpu.is_some(), gpu.as_deref().filter(|v| *v != "1").unwrap_or(""))
    }

    /// The graphics card, if this is one.
    pub fn card(self) -> Option<GraphicsCard> {
        cards().into_iter().find(|c| self == Device::Gpu(c.luid))
    }

    /// The sherpa-onnx provider for this device. For a graphics card it also
    /// tells the patched sherpa-onnx which adapter to open (read when a model
    /// is loaded, see scripts/build-sherpa-directml.ps1).
    pub fn provider(self) -> String {
        match self {
            Device::Cpu => "cpu".into(),
            Device::Gpu(luid) => {
                set_c_runtime_variable("SHERPA_ONNX_DML_ADAPTER_LUID", &format!("{:x}:{:x}", luid >> 32, luid & 0xffff_ffff));
                "directml".into()
            }
        }
    }
}

/// Sets an environment variable where sherpa-onnx's `getenv` sees it: in the
/// C runtime's copy of the environment, taken at startup. `std::env::set_var`
/// only changes the process environment, which sherpa-onnx never saw.
/// sherpa-onnx is linked into the executable with the same C runtime.
fn set_c_runtime_variable(name: &str, value: &str) {
    extern "C" {
        fn _putenv_s(name: *const std::ffi::c_char, value: *const std::ffi::c_char) -> i32;
    }
    let (name, value) = (std::ffi::CString::new(name).unwrap(), std::ffi::CString::new(value).unwrap());
    unsafe {
        _putenv_s(name.as_ptr(), value.as_ptr());
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsCard {
    /// Locally unique id of the adapter (high and low part).
    #[serde(skip)]
    pub luid: u64,
    pub name: String,
    /// Dedicated video memory in bytes.
    pub memory: u64,
}

/// The hardware graphics adapters, in DXGI's order.
pub fn cards() -> Vec<GraphicsCard> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new() };
    let mut cards = Vec::new();
    for index in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else { break };
        let Ok(description) = (unsafe { adapter.GetDesc1() }) else { continue };
        // The "Microsoft Basic Render Driver" draws on the processor.
        if description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let length = description.Description.iter().position(|c| *c == 0).unwrap_or(description.Description.len());
        let luid = description.AdapterLuid;
        cards.push(GraphicsCard {
            luid: (luid.HighPart as u32 as u64) << 32 | luid.LowPart as u64,
            name: String::from_utf16_lossy(&description.Description[..length]).trim().to_string(),
            memory: description.DedicatedVideoMemory as u64,
        });
    }
    cards
}

/// Marks work on the graphics card. A crash in the graphics driver takes the
/// whole app down; the marker left behind turns the graphics card off at the
/// next start, so Scripta doesn't crash again and again. Nested guards leave
/// the marker to the outermost one.
pub struct Guard(Option<PathBuf>);

impl Guard {
    pub fn new(device: Device) -> Guard {
        let path = paths::gpu_marker_file();
        if device == Device::Cpu || path.exists() {
            return Guard(None);
        }
        let _ = std::fs::write(&path, b"");
        Guard(Some(path))
    }

    /// True (once) if the last run ended during work on the graphics card.
    pub fn take_crash() -> bool {
        std::fs::remove_file(paths::gpu_marker_file()).is_ok()
    }

    /// A clean exit during work on the graphics card is no crash.
    pub fn clear() {
        let _ = std::fs::remove_file(paths::gpu_marker_file());
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}
