use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Context;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize tracing: always append to `{data_dir}/logs/gateway.log`.
/// When `log_to_stderr` is true (foreground), also mirror logs to stderr.
pub fn init(data_dir: &std::path::Path, log_to_stderr: bool) -> anyhow::Result<PathBuf> {
    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)
        .with_context(|| format!("create logs dir {}", logs_dir.display()))?;

    let log_path = logs_dir.join("gateway.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open log file {}", log_path.display()))?;

    let filter = EnvFilter::from_default_env()
        .add_directive("token_router=info".parse().context("log filter")?);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(file));

    if !tracing::dispatcher::has_been_set() {
        let registry = tracing_subscriber::registry().with(filter).with(file_layer);
        if log_to_stderr {
            registry.with(fmt::layer()).try_init()?;
        } else {
            registry.try_init()?;
        }
    }

    Ok(log_path)
}
