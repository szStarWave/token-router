use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use serde::Serialize;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const LOG_TAIL_BYTES: u64 = 128 * 1024;
pub const LOG_CHUNK_BYTES: u64 = 64 * 1024;
const LOG_CHUNK_CAP: u64 = 512 * 1024;

#[derive(Serialize, Clone)]
pub struct LogLine {
    pub level: &'static str,
    pub text: String,
}

#[derive(Serialize, Clone)]
pub struct LogsTail {
    pub offset: u64,
    pub next_offset: u64,
    pub reset: bool,
    pub lines: Vec<LogLine>,
}

/// Initialize tracing: always append to `{data_dir}/logs/gateway.log`.
/// When `log_to_stderr` is true (foreground), also mirror logs to stderr.
pub fn init(data_dir: &Path, log_to_stderr: bool) -> anyhow::Result<PathBuf> {
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

/// Read incremental tail bytes from `gateway.log`.
pub fn read_log_tail(
    path: &Path,
    offset: Option<u64>,
    max_bytes: Option<u64>,
) -> anyhow::Result<LogsTail> {
    let max_bytes = max_bytes
        .unwrap_or(LOG_CHUNK_BYTES)
        .clamp(1, LOG_CHUNK_CAP);

    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogsTail {
                offset: 0,
                next_offset: 0,
                reset: false,
                lines: Vec::new(),
            });
        }
        Err(e) => return Err(e.into()),
    };

    let file_len = meta.len();
    let mut reset = false;
    let start = match offset {
        Some(off) if off > file_len => {
            reset = true;
            file_len.saturating_sub(max_bytes.max(LOG_TAIL_BYTES))
        }
        Some(off) => off,
        None => file_len.saturating_sub(max_bytes.max(LOG_TAIL_BYTES)),
    };

    if start >= file_len {
        return Ok(LogsTail {
            offset: start,
            next_offset: file_len,
            reset,
            lines: Vec::new(),
        });
    }

    let end = (start + max_bytes).min(file_len);
    let lines = read_log_bytes(path, start, end)?;

    Ok(LogsTail {
        offset: start,
        next_offset: file_len,
        reset,
        lines,
    })
}

/// Read a chunk of log bytes ending at `before` (exclusive upper bound).
/// Used for lazy-loading older lines when scrolling up in the UI.
pub fn read_log_before(
    path: &Path,
    before: u64,
    max_bytes: Option<u64>,
) -> anyhow::Result<LogsTail> {
    let max_bytes = max_bytes
        .unwrap_or(LOG_CHUNK_BYTES)
        .clamp(1, LOG_CHUNK_CAP);

    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogsTail {
                offset: 0,
                next_offset: 0,
                reset: false,
                lines: Vec::new(),
            });
        }
        Err(e) => return Err(e.into()),
    };

    let file_len = meta.len();
    let before = before.min(file_len);
    if before == 0 {
        return Ok(LogsTail {
            offset: 0,
            next_offset: 0,
            reset: false,
            lines: Vec::new(),
        });
    }

    let start = before.saturating_sub(max_bytes);
    let lines = read_log_bytes(path, start, before)?;

    Ok(LogsTail {
        offset: start,
        next_offset: before,
        reset: false,
        lines,
    })
}

fn read_log_bytes(path: &Path, start: u64, end: u64) -> anyhow::Result<Vec<LogLine>> {
    if start >= end {
        return Ok(Vec::new());
    }

    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let byte_len = (end - start) as usize;
    let mut buf = vec![0u8; byte_len];
    let read_len = file.read(&mut buf)?;
    buf.truncate(read_len);

    let text = String::from_utf8_lossy(&buf);
    let skip_first = start > 0;
    Ok(text
        .lines()
        .skip(if skip_first { 1 } else { 0 })
        .filter(|line| !line.is_empty())
        .map(|line| LogLine {
            level: classify_log_level(line),
            text: line.to_string(),
        })
        .collect())
}

fn classify_log_level(line: &str) -> &'static str {
    if line.contains(" ERROR ") {
        "err"
    } else if line.contains(" WARN ") {
        "warn"
    } else {
        "info"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_log_levels() {
        assert_eq!(
            classify_log_level("2025-01-01T00:00:00.000000Z  INFO token_router: started"),
            "info"
        );
        assert_eq!(
            classify_log_level("2025-01-01T00:00:00.000000Z  WARN token_router: retry"),
            "warn"
        );
        assert_eq!(
            classify_log_level("2025-01-01T00:00:00.000000Z  ERROR token_router: failed"),
            "err"
        );
    }

    #[test]
    fn read_log_before_returns_older_chunk() {
        let path = std::env::temp_dir().join(format!(
            "token-router-log-before-{}.log",
            std::process::id()
        ));
        let mut content = String::new();
        for i in 0..500 {
            content.push_str(&format!("2025-01-01T00:00:00.000000Z  INFO token_router: line {i}\n"));
        }
        std::fs::write(&path, &content).unwrap();
        let file_len = content.len() as u64;
        let before = file_len / 2;

        let older = read_log_before(&path, before, Some(512)).unwrap();
        assert!(!older.lines.is_empty());
        assert!(older.offset < before);
        assert_eq!(older.next_offset, before);

        let _ = std::fs::remove_file(path);
    }
}
