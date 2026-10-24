//! Speed and memory of transcription and diarization for a long recording
//! (development tool). Repeats the given file up to the wanted length.
//!
//! cargo run --release --example benchmark -- file.m4a [minutes]
//! SCRIPTA_GPU=1 (or a card's name) runs speech recognition on the graphics
//! card; speaker separation stays on the processor, as in the app.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use scripta_lib::audio::{mf, MODEL_RATE};
use scripta_lib::engine::{thread_count, SpeakerEngine, SpeechEngine};
use scripta_lib::gpu::Device;

fn peak_memory_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb);
    }
    counters.PeakWorkingSetSize as f64 / 1e6
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    tracing_subscriber::fmt().with_target(false).init();
    let minutes: f64 = args.get(1).and_then(|m| m.parse().ok()).unwrap_or(60.0);
    let started = Instant::now();
    let clip = mf::decode_to_vec(std::path::Path::new(&args[0]), MODEL_RATE, |_| {})?;
    println!("file read in {:.1} s ({:.1} min of audio)", started.elapsed().as_secs_f64(), clip.len() as f64 / MODEL_RATE as f64 / 60.0);
    let wanted = (minutes * 60.0 * MODEL_RATE as f64) as usize;
    let mut samples = Vec::with_capacity(wanted);
    while samples.len() < wanted {
        samples.extend_from_slice(&clip[..clip.len().min(wanted - samples.len())]);
    }
    let seconds = samples.len() as f64 / MODEL_RATE as f64;
    let device = Device::from_env();
    let card = device.card().map(|c| c.name).unwrap_or_else(|| "processor".into());
    println!("audio {:.1} min on {card}, {} threads, memory before models {:.0} MB", seconds / 60.0, thread_count(), peak_memory_mb());

    let started = Instant::now();
    let speech = SpeechEngine::load(device)?;
    println!("speech model loaded in {:.1} s", started.elapsed().as_secs_f64());
    let started = Instant::now();
    let words = speech.transcribe(&samples, &AtomicBool::new(false), |_| {})?;
    let asr = started.elapsed().as_secs_f64();
    println!("transcription: {:.1} s ({:.0}× real time), {} words, peak memory {:.0} MB", asr, seconds / asr, words.len(), peak_memory_mb());
    drop(speech);

    let started = Instant::now();
    let speakers = SpeakerEngine::load(Device::Cpu)?;
    let diarization = speakers.diarize(&samples, None)?;
    let diarize = started.elapsed().as_secs_f64();
    println!(
        "diarization: {:.1} s ({:.0}× real time), {} speakers, {} turns, peak memory {:.0} MB",
        diarize,
        seconds / diarize,
        diarization.speakers().len(),
        diarization.turns.len(),
        peak_memory_mb()
    );
    println!("total {:.1} min for {:.0} min of audio", (asr + diarize) / 60.0, seconds / 60.0);
    Ok(())
}
