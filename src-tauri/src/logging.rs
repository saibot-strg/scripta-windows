//! Log files in the `Logs` folder of the app data, one per day, two weeks kept.
//! Logs never contain transcript text, names or voice data; IDs and
//! numbers only.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Starts logging; keep the guard alive for the lifetime of the app.
pub fn start() -> Option<WorkerGuard> {
    let dir = crate::paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("scripta")
        .filename_suffix("log")
        .max_log_files(14)
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("SCRIPTA_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let file_layer = tracing_subscriber::fmt::layer().with_writer(writer).with_ansi(false).with_target(false);
    let registry = tracing_subscriber::registry().with(filter).with(file_layer);
    if cfg!(debug_assertions) {
        let _ = registry.with(tracing_subscriber::fmt::layer().with_target(false)).try_init();
    } else {
        let _ = registry.try_init();
    }
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(panic = %info, "panic");
    }));
    Some(guard)
}
