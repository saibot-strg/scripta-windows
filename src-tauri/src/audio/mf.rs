//! Media Foundation: reads the audio of any file Windows can play (mp3, m4a,
//! wav, wma, flac, the sound of videos …) and writes AAC (.m4a).
//!
//! Every function initializes COM (multithreaded) and Media Foundation on
//! the calling thread; call them from worker threads, not the UI thread.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use super::resample::Resampler;

/// Keeps Media Foundation started for the lifetime of the guard.
struct MediaFoundation;

impl MediaFoundation {
    fn start() -> Result<Self> {
        unsafe {
            // S_FALSE / RPC_E_CHANGED_MODE: COM is already initialized on this thread.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup")?;
        }
        Ok(Self)
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

const FIRST_AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

/// Format of the decoded audio.
#[derive(Clone, Copy, Debug)]
pub struct DecodedFormat {
    pub sample_rate: u32,
    pub channels: u32,
}

fn open_reader(path: &Path) -> Result<(IMFSourceReader, DecodedFormat)> {
    unsafe {
        let url = HSTRING::from(path.as_os_str());
        let reader: IMFSourceReader =
            MFCreateSourceReaderFromURL(&url, None::<&IMFAttributes>).with_context(|| format!("opening {}", path.display()))?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(FIRST_AUDIO, true).map_err(|_| anyhow!("no audio track"))?;
        let wanted: IMFMediaType = MFCreateMediaType()?;
        wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        wanted.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
        reader.SetCurrentMediaType(FIRST_AUDIO, None, &wanted).context("no decoder for this audio")?;
        let current = reader.GetCurrentMediaType(FIRST_AUDIO)?;
        let format = DecodedFormat {
            sample_rate: current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?,
            channels: current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?.max(1),
        };
        Ok((reader, format))
    }
}

/// Duration in seconds as the container reports it.
fn reported_duration(reader: &IMFSourceReader) -> Option<f64> {
    unsafe {
        let value = reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION).ok()?;
        let inner = &value.Anonymous.Anonymous;
        if inner.vt == windows::Win32::System::Variant::VT_UI8 {
            Some(inner.Anonymous.uhVal as f64 / 10_000_000.0)
        } else {
            None
        }
    }
}

/// True if the file has an audio track Windows can decode.
pub fn has_audio(path: &Path) -> bool {
    let Ok(_mf) = MediaFoundation::start() else { return false };
    open_reader(path).is_ok()
}

/// Duration of the audio in seconds; decodes the file if the container
/// doesn't say.
pub fn duration(path: &Path) -> Result<f64> {
    let _mf = MediaFoundation::start()?;
    let (reader, format) = open_reader(path)?;
    if let Some(duration) = reported_duration(&reader) {
        if duration > 0.0 {
            return Ok(duration);
        }
    }
    let _ = format;
    let mut seconds = 0.0;
    read_all(&reader, |interleaved, format| {
        seconds += (interleaved.len() / format.channels as usize) as f64 / format.sample_rate as f64;
        Ok(true)
    })?;
    Ok(seconds)
}

fn current_format(reader: &IMFSourceReader) -> Result<DecodedFormat> {
    unsafe {
        let current = reader.GetCurrentMediaType(FIRST_AUDIO)?;
        Ok(DecodedFormat {
            sample_rate: current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?,
            channels: current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?.max(1),
        })
    }
}

/// Calls `chunk` with interleaved float samples and their format until the
/// end of the file or until it returns false. The format may change once
/// the decoder has seen the first frame (common with AAC).
fn read_all(reader: &IMFSourceReader, mut chunk: impl FnMut(&[f32], DecodedFormat) -> Result<bool>) -> Result<()> {
    let mut format = current_format(reader)?;
    loop {
        let mut flags: u32 = 0;
        let mut sample: Option<IMFSample> = None;
        unsafe {
            reader.ReadSample(FIRST_AUDIO, 0, None, Some(&mut flags as *mut u32), None, Some(&mut sample as *mut _))?;
        }
        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
            format = current_format(reader)?;
        }
        if let Some(sample) = sample {
            unsafe {
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut length: u32 = 0;
                buffer.Lock(&mut data, None, Some(&mut length as *mut u32))?;
                let floats = std::slice::from_raw_parts(data as *const f32, length as usize / 4).to_vec();
                buffer.Unlock()?;
                if !chunk(&floats, format)? {
                    return Ok(());
                }
            }
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(());
        }
    }
}

/// Decodes the whole file to mono at `rate`, passing chunks to `consume`
/// as they are ready. `progress` gets 0...1 (by time, if the duration is
/// known). Return false from `consume` to stop early. Returns the duration.
pub fn decode_mono(
    path: &Path,
    rate: u32,
    mut consume: impl FnMut(&[f32]) -> Result<bool>,
    mut progress: impl FnMut(f64),
) -> Result<f64> {
    let _mf = MediaFoundation::start()?;
    let (reader, _) = open_reader(path)?;
    let total = reported_duration(&reader).filter(|d| *d > 0.0);
    let mut resampler: Option<(u32, Resampler)> = None;
    let mut out = Vec::new();
    let mut seconds = 0.0;
    let mut stopped = false;
    read_all(&reader, |interleaved, format| {
        let mono = super::downmix(interleaved, format.channels as usize);
        seconds += mono.len() as f64 / format.sample_rate as f64;
        out.clear();
        match &mut resampler {
            Some((from, current)) if *from == format.sample_rate => current.process(&mono, &mut out),
            _ => {
                if let Some((_, previous)) = resampler.as_mut() {
                    previous.finish(&mut out);
                }
                let mut next = Resampler::new(format.sample_rate, rate);
                next.process(&mono, &mut out);
                resampler = Some((format.sample_rate, next));
            }
        }
        if !consume(&out)? {
            stopped = true;
            return Ok(false);
        }
        if let Some(total) = total {
            progress((seconds / total).min(1.0));
        }
        Ok(true)
    })?;
    if !stopped {
        if let Some((_, mut last)) = resampler {
            out.clear();
            last.finish(&mut out);
            consume(&out)?;
        }
    }
    progress(1.0);
    Ok(seconds)
}

/// The whole file as mono samples at `rate` (e.g. 16 kHz for the models).
pub fn decode_to_vec(path: &Path, rate: u32, progress: impl FnMut(f64)) -> Result<Vec<f32>> {
    let mut samples = Vec::new();
    decode_mono(
        path,
        rate,
        |chunk| {
            samples.extend_from_slice(chunk);
            Ok(true)
        },
        progress,
    )?;
    Ok(samples)
}

/// Writes mono 16-bit PCM chunks at `rate` as AAC in an MPEG-4 file.
pub struct AacWriter {
    writer: IMFSinkWriter,
    stream: u32,
    rate: u32,
    frames: u64,
    // Last: fields drop in order, Media Foundation must outlive the writer.
    _mf: MediaFoundation,
}

impl AacWriter {
    /// `bit_rate`: 96_000, 128_000, 160_000 or 192_000 (what the Windows
    /// AAC encoder supports).
    pub fn create(path: &Path, rate: u32, bit_rate: u32) -> Result<Self> {
        let mf = MediaFoundation::start()?;
        unsafe {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.ok_or_else(|| anyhow!("no attributes"))?;
            attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            let url = HSTRING::from(path.as_os_str());
            let writer = MFCreateSinkWriterFromURL(&url, None::<&IMFByteStream>, &attributes)
                .with_context(|| format!("creating {}", path.display()))?;

            let output: IMFMediaType = MFCreateMediaType()?;
            output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            output.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            output.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
            output.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)?;
            output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bit_rate / 8)?;
            let stream = writer.AddStream(&output).context("AAC encoder rejected the format")?;

            let input: IMFMediaType = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
            input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)?;
            input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 2)?;
            input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * 2)?;
            writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>)?;
            writer.BeginWriting()?;
            Ok(Self { writer, stream, rate, frames: 0, _mf: mf })
        }
    }

    pub fn write(&mut self, samples: &[i16]) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        unsafe {
            let bytes = (samples.len() * 2) as u32;
            let buffer = MFCreateMemoryBuffer(bytes)?;
            let mut data: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(samples.as_ptr() as *const u8, data, bytes as usize);
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let time = (self.frames as i128 * 10_000_000 / self.rate as i128) as i64;
            let duration = (samples.len() as i128 * 10_000_000 / self.rate as i128) as i64;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(duration)?;
            self.writer.WriteSample(self.stream, &sample)?;
        }
        self.frames += samples.len() as u64;
        Ok(())
    }

    pub fn finish(self) -> Result<f64> {
        unsafe {
            self.writer.Finalize().context("finishing the AAC file")?;
        }
        Ok(self.frames as f64 / self.rate as f64)
    }
}

/// Encodes a 16-bit WAV file to AAC. The output is written under a
/// temporary name and renamed when complete.
pub fn encode_wav_to_aac(source: &Path, destination: &Path, bit_rate: u32) -> Result<f64> {
    let mut reader = hound::WavReader::open(source)?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err(anyhow!("unexpected WAV format"));
    }
    let temporary = destination.with_file_name(format!("encoding-{}.m4a", uuid::Uuid::new_v4()));
    let result = (|| -> Result<f64> {
        let mut writer = AacWriter::create(&temporary, spec.sample_rate, bit_rate)?;
        let mut chunk = Vec::with_capacity(spec.sample_rate as usize);
        for sample in reader.samples::<i16>() {
            chunk.push(sample?);
            if chunk.len() == spec.sample_rate as usize {
                writer.write(&chunk)?;
                chunk.clear();
            }
        }
        writer.write(&chunk)?;
        let duration = writer.finish()?;
        std::fs::rename(&temporary, destination)?;
        Ok(duration)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Extracts the audio of any file (e.g. a video) as mono AAC at 48 kHz.
pub fn extract_audio_to_aac(source: &Path, destination: &Path, bit_rate: u32) -> Result<f64> {
    let temporary = destination.with_file_name(format!("extracting-{}.m4a", uuid::Uuid::new_v4()));
    let result = (|| -> Result<f64> {
        let mut writer = AacWriter::create(&temporary, super::CAPTURE_RATE, bit_rate)?;
        let mut pcm = Vec::new();
        decode_mono(
            source,
            super::CAPTURE_RATE,
            |chunk| {
                pcm.clear();
                pcm.extend(chunk.iter().map(|x| (x.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16));
                writer.write(&pcm)?;
                Ok(true)
            },
            |_| {},
        )?;
        let duration = writer.finish()?;
        std::fs::rename(&temporary, destination)?;
        Ok(duration)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::write_mono;

    fn tone(seconds: f64) -> Vec<f32> {
        (0..(48_000.0 * seconds) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 48_000.0).sin() as f32 * 0.3)
            .collect()
    }

    #[test]
    fn wav_is_decoded_and_resampled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_mono(&path, &tone(2.0), 48_000).unwrap();
        let samples = decode_to_vec(&path, 16_000, |_| {}).unwrap();
        assert_eq!(samples.len(), 32_000);
        assert!((duration(&path).unwrap() - 2.0).abs() < 0.01);
    }

    #[test]
    fn aac_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("tone.wav");
        let m4a = dir.path().join("tone.m4a");
        write_mono(&wav, &tone(3.0), 48_000).unwrap();
        let written = encode_wav_to_aac(&wav, &m4a, 96_000).unwrap();
        assert!((written - 3.0).abs() < 0.01);
        let size = std::fs::metadata(&m4a).unwrap().len();
        assert!(size > 10_000 && size < 100_000, "{size}");
        let reported = duration(&m4a).unwrap();
        assert!((reported - 3.0).abs() < 0.1, "{reported}");
        let decoded = decode_to_vec(&m4a, 16_000, |_| {}).unwrap();
        assert!((decoded.len() as i64 - 48_000).abs() < 3_000, "{}", decoded.len());
        let rms = (decoded[8_000..40_000].iter().map(|x| x * x).sum::<f32>() / 32_000.0).sqrt();
        assert!((rms - 0.212).abs() < 0.03, "{rms}");
    }
}
