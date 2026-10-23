//! Downloads the speech and speaker models into the app's model folder
//! (development and CI; the app downloads them itself on first launch).
//!
//! cargo run --example models

use std::io::Write;
use std::sync::atomic::AtomicBool;

use scripta_lib::models::ModelKind;

fn main() -> anyhow::Result<()> {
    let cancel = AtomicBool::new(false);
    for kind in [ModelKind::Speech, ModelKind::Speakers] {
        if kind.is_installed() {
            println!("{kind:?}: installed");
            continue;
        }
        let mut last = -1i64;
        kind.download(&cancel, |fraction| {
            let percent = (fraction * 100.0) as i64;
            if percent != last {
                last = percent;
                print!("\r{kind:?}: {percent}%");
                let _ = std::io::stdout().flush();
            }
        })?;
        println!();
    }
    Ok(())
}
