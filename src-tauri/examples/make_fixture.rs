//! Encodes a WAV file as AAC at 48 kHz for the test fixtures.
//!
//! cargo run --release --example make_fixture -- in.wav out.m4a

use scripta_lib::audio::{mf::AacWriter, resample::resample, wav};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut samples = Vec::new();
    let rate = wav::read_chunks(std::path::Path::new(&args[0]), |chunk| {
        samples.extend_from_slice(chunk);
        Ok(())
    })?;
    let samples = resample(&samples, rate, 48_000);
    let mut writer = AacWriter::create(std::path::Path::new(&args[1]), 48_000, 96_000)?;
    let pcm: Vec<i16> = samples.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
    for chunk in pcm.chunks(48_000) {
        writer.write(chunk)?;
    }
    println!("{:.1} s", writer.finish()?);
    Ok(())
}
