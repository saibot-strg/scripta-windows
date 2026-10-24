//! WASAPI capture: a microphone, all system audio (loopback of the default
//! output), or the audio of one app and its child processes (process
//! loopback, Windows 10 2004 and later).
//!
//! Each source runs on its own thread and delivers mono 48 kHz float
//! buffers with the capture time of their first frame (100-ns units on the
//! QPC clock), so the tracks of a meeting can be laid on one timeline.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Result};
use wasapi::{AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

use super::CAPTURE_RATE;

#[derive(Clone, Debug, PartialEq)]
pub enum CaptureSource {
    /// A microphone by endpoint ID; None = the system default.
    Microphone(Option<String>),
    /// Everything played on the default output device.
    SystemAudio,
    /// One app (by process ID of its top process) and its child processes.
    Application(u32),
}

/// Now on the QPC clock, in 100-ns units (like WASAPI timestamps).
pub fn now_100ns() -> u64 {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    let mut counter = 0i64;
    let mut frequency = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    (counter as u128 * 10_000_000 / frequency.max(1) as u128) as u64
}

/// A running capture; stops when dropped.
pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    /// Starts capturing. `on_audio` gets mono 48 kHz samples and the time of
    /// their first frame; `on_error` is called once if capturing fails later
    /// (e.g. the headset was unplugged). Fails if the source can't be opened.
    pub fn start(
        source: CaptureSource,
        on_audio: impl FnMut(&[f32], u64) + Send + 'static,
        on_error: impl FnOnce(String) + Send + 'static,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread_stop = stop.clone();
        let name = match source {
            CaptureSource::Microphone(_) => "capture-microphone",
            _ => "capture-system",
        };
        let thread = std::thread::Builder::new().name(name.into()).spawn(move || {
            crate::priority::raise_current_thread();
            let result = run(source, &thread_stop, on_audio, |ready| {
                let _ = ready_tx.send(ready);
            });
            if let Err(error) = result {
                tracing::error!(%error, "capture stopped");
                if !thread_stop.load(Ordering::Relaxed) {
                    on_error(error.to_string());
                }
            }
        })?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self { stop, thread: Some(thread) }),
            Ok(Err(message)) => {
                stop.store(true, Ordering::Relaxed);
                let _ = thread.join();
                Err(anyhow!(message))
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err(anyhow!("the audio device did not respond"))
            }
        }
    }

    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.halt();
    }
}

fn open_client(source: &CaptureSource) -> Result<(AudioClient, usize)> {
    match source {
        CaptureSource::Microphone(id) => {
            let enumerator = DeviceEnumerator::new()?;
            let device = match id {
                Some(id) => enumerator.get_device(id)?,
                None => enumerator.get_default_device(&Direction::Capture)?,
            };
            Ok((device.get_iaudioclient()?, 1))
        }
        CaptureSource::SystemAudio => {
            let enumerator = DeviceEnumerator::new()?;
            let device = enumerator.get_default_device(&Direction::Render)?;
            Ok((device.get_iaudioclient()?, 2))
        }
        CaptureSource::Application(process_id) => Ok((AudioClient::new_application_loopback_client(*process_id, true)?, 2)),
    }
}

fn run(
    source: CaptureSource,
    stop: &AtomicBool,
    mut on_audio: impl FnMut(&[f32], u64),
    mut ready: impl FnMut(Result<(), String>),
) -> Result<()> {
    let _ = wasapi::initialize_mta();
    let setup = (|| -> Result<_> {
        let (mut client, channels) = open_client(&source)?;
        let format = WaveFormat::new(32, 32, &SampleType::Float, CAPTURE_RATE as usize, channels, None);
        let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
        client.initialize_client(&format, &Direction::Capture, &mode)?;
        let event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;
        Ok((client, event, capture, channels))
    })();
    let (client, event, capture, channels) = match setup {
        Ok(parts) => {
            ready(Ok(()));
            parts
        }
        Err(error) => {
            let message = error.to_string();
            ready(Err(message));
            return Ok(());
        }
    };
    tracing::info!(?source, "capture started");

    let block_align = 4 * channels;
    let mut bytes: VecDeque<u8> = VecDeque::new();
    let mut floats: Vec<f32> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        // Loopback sources only signal while something plays; a timeout is normal.
        if event.wait_for_event(200).is_err() {
            continue;
        }
        loop {
            let frames = capture.get_next_packet_size()?.unwrap_or(0);
            if frames == 0 {
                break;
            }
            bytes.clear();
            let info = capture.read_from_device_to_deque(&mut bytes)?;
            let count = bytes.len() / block_align;
            if count == 0 {
                continue;
            }
            floats.clear();
            if info.flags.silent {
                floats.resize(count * channels, 0.0);
            } else {
                let (front, back) = bytes.as_slices();
                let mut raw = Vec::with_capacity(bytes.len());
                raw.extend_from_slice(front);
                raw.extend_from_slice(back);
                floats.extend(raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            }
            let mono = super::downmix(&floats, channels);
            let duration = mono.len() as u64 * 10_000_000 / CAPTURE_RATE as u64;
            let timestamp = if info.timestamp > 0 && !info.flags.timestamp_error {
                info.timestamp
            } else {
                now_100ns().saturating_sub(duration)
            };
            on_audio(&mono, timestamp);
        }
    }
    let _ = client.stop_stream();
    tracing::info!(?source, "capture stopped");
    Ok(())
}
