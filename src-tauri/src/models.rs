//! The speech models: downloaded once from Hugging Face, then everything
//! runs offline.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::gpu::Device;
use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelKind {
    /// Parakeet (speech to text, int8) plus the voice activity detector.
    Speech,
    /// Parakeet in full precision for the graphics card (see `on`).
    SpeechFull,
    /// Speaker segmentation and voice embeddings.
    Speakers,
}

struct ModelFile {
    repo: &'static str,
    remote: &'static str,
    local: &'static str,
    size: u64,
}

const SPEECH_FILES: &[ModelFile] = &[
    ModelFile {
        repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        remote: "encoder.int8.onnx",
        local: "encoder.int8.onnx",
        size: 652_184_281,
    },
    ModelFile {
        repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        remote: "decoder.int8.onnx",
        local: "decoder.int8.onnx",
        size: 11_845_275,
    },
    ModelFile {
        repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        remote: "joiner.int8.onnx",
        local: "joiner.int8.onnx",
        size: 6_355_277,
    },
    ModelFile {
        repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        remote: "tokens.txt",
        local: "tokens.txt",
        size: 93_939,
    },
    ModelFile { repo: "csukuangfj/vad", remote: "silero_vad.onnx", local: "silero_vad.onnx", size: 1_807_522 },
];

const SPEECH_FULL_FILES: &[ModelFile] = &[
    ModelFile { repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3", remote: "encoder.onnx", local: "encoder.onnx", size: 41_766_257 },
    ModelFile {
        repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3",
        remote: "encoder.weights",
        local: "encoder.weights",
        size: 2_435_420_160,
    },
    ModelFile { repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3", remote: "decoder.onnx", local: "decoder.onnx", size: 47_233_743 },
    ModelFile { repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3", remote: "joiner.onnx", local: "joiner.onnx", size: 25_286_330 },
    ModelFile { repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3", remote: "tokens.txt", local: "tokens.txt", size: 93_939 },
    ModelFile { repo: "csukuangfj/vad", remote: "silero_vad.onnx", local: "silero_vad.onnx", size: 1_807_522 },
];

const SPEAKER_FILES: &[ModelFile] = &[
    ModelFile {
        repo: "csukuangfj/sherpa-onnx-pyannote-segmentation-3-0",
        remote: "model.onnx",
        local: "segmentation.onnx",
        size: 5_992_913,
    },
    // NVIDIA TitaNet small: of the embedding models tried on synthetic
    // dialogues (clean and call-like, German and English), it separated all
    // of them over the widest threshold range (see docs/speaker-models.md).
    ModelFile {
        repo: "csukuangfj/speaker-embedding-models",
        remote: "nemo_en_titanet_small.onnx",
        local: "titanet-small.onnx",
        size: 40_257_283,
    },
];

/// Stored with each transcript.
pub const ENGINE_IDENTIFIER: &str = "parakeet-tdt-0.6b-v3";

impl ModelKind {
    /// The model behind a kind on a device. On a graphics card speech uses
    /// Parakeet in full precision: with DirectML the int8 model was slower
    /// than the processor, and it left out whole sentences the full model
    /// recognized (433 against 491 words in 10 minutes of a Teams meeting).
    /// Speaker separation always runs on the processor.
    pub fn on(self, device: Device) -> ModelKind {
        match (self, device) {
            (Self::Speech, Device::Gpu(_)) => Self::SpeechFull,
            _ => self,
        }
    }

    fn files(self) -> &'static [ModelFile] {
        match self {
            Self::Speech => SPEECH_FILES,
            Self::SpeechFull => SPEECH_FULL_FILES,
            Self::Speakers => SPEAKER_FILES,
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::Speech => "parakeet-tdt-0.6b-v3-int8",
            Self::SpeechFull => "parakeet-tdt-0.6b-v3",
            Self::Speakers => "speaker-diarization",
        }
    }

    pub fn directory(self) -> PathBuf {
        paths::models_dir().join(self.folder())
    }

    pub fn file(self, local: &str) -> PathBuf {
        self.directory().join(local)
    }

    /// Download size in bytes.
    pub fn size(self) -> u64 {
        self.files().iter().map(|f| f.size).sum()
    }

    /// True if all files are on disk (no network needed).
    pub fn is_installed(self) -> bool {
        self.directory().join(COMPLETE_MARKER).exists() && self.files().iter().all(|f| self.file(f.local).exists())
    }

    /// Size on disk, if installed.
    pub fn installed_size(self) -> Option<u64> {
        self.is_installed().then(|| self.files().iter().filter_map(|f| std::fs::metadata(self.file(f.local)).ok()).map(|m| m.len()).sum())
    }

    pub fn delete(self) -> Result<()> {
        let directory = self.directory();
        if directory.exists() {
            std::fs::remove_dir_all(&directory)?;
        }
        Ok(())
    }

    /// Downloads the missing files. `progress` gets 0...1 over all files.
    pub fn download(self, cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<()> {
        let directory = self.directory();
        std::fs::create_dir_all(&directory)?;
        let total = self.size().max(1);
        let mut done = 0u64;
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(None)
            .user_agent(concat!("Scripta-Windows/", env!("CARGO_PKG_VERSION")))
            .build()?;
        for file in self.files() {
            let target = directory.join(file.local);
            if target.exists() {
                done += file.size;
                progress(done as f64 / total as f64);
                continue;
            }
            let url = format!("https://huggingface.co/{}/resolve/main/{}", file.repo, file.remote);
            tracing::info!(file = file.local, "downloading model file");
            download_file(&client, &url, &target, cancel, |bytes| {
                progress((done + bytes.min(file.size)) as f64 / total as f64);
            })
            .with_context(|| format!("downloading {}", file.remote))?;
            done += file.size;
        }
        // Files of earlier model versions.
        let wanted: Vec<&str> = self.files().iter().map(|f| f.local).collect();
        for entry in std::fs::read_dir(&directory)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != COMPLETE_MARKER && !wanted.contains(&name.as_str()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        std::fs::write(directory.join(COMPLETE_MARKER), b"ok")?;
        progress(1.0);
        Ok(())
    }
}

const COMPLETE_MARKER: &str = "complete";

fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    target: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let partial = target.with_extension("part");
    let result = (|| -> Result<()> {
        let mut response = client.get(url).send()?.error_for_status()?;
        let expected = response.content_length();
        let mut file = std::io::BufWriter::new(std::fs::File::create(&partial)?);
        let mut buffer = vec![0u8; 1 << 16];
        let mut written = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("cancelled"));
            }
            let count = response.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            file.write_all(&buffer[..count])?;
            written += count as u64;
            progress(written);
        }
        file.flush()?;
        drop(file);
        if let Some(expected) = expected {
            if written != expected {
                return Err(anyhow!("incomplete download ({written} of {expected} bytes)"));
            }
        }
        std::fs::rename(&partial, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}
