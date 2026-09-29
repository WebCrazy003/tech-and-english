use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Logs go to stdout (dev) and to a daily-rotated file (5 files kept).
/// Never log article bodies, transcripts or LLM output — ids, counts and timings only.
/// Keep the returned guard alive for the life of the app.
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tech_english_lib=debug"));

    let file = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("app")
        .filename_suffix("log")
        .max_log_files(5)
        .build(log_dir)
        .ok();

    match file {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .with(fmt::layer().with_ansi(false).with_writer(writer))
                .try_init()
                .ok();
            Some(guard)
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .try_init()
                .ok();
            None
        }
    }
}
