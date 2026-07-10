use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use time::OffsetDateTime;

use anyhow::Context;
use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};
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

#[derive(Debug, Clone, Copy)]
pub struct LogRotateConfig {
    pub max_size_mb: u64,
    pub max_files: usize,
}

impl LogRotateConfig {
    pub fn rotation_enabled(self) -> bool {
        self.max_size_mb > 0 && self.max_files > 1
    }

    fn max_size_bytes(self) -> u64 {
        self.max_size_mb.saturating_mul(1024 * 1024)
    }

    fn append_count(self) -> usize {
        self.max_files.saturating_sub(1)
    }
}

/// Initialize tracing: append to `{data_dir}/logs/gateway.log` (with optional size rotation).
/// When `log_to_stderr` is true (foreground), also mirror logs to stderr.
pub fn init(
    data_dir: &Path,
    log_to_stderr: bool,
    rotate: LogRotateConfig,
) -> anyhow::Result<PathBuf> {
    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)
        .with_context(|| format!("create logs dir {}", logs_dir.display()))?;

    let log_path = logs_dir.join("gateway.log");
    let filter = EnvFilter::from_default_env()
        .add_directive("token_router=info".parse().context("log filter")?);

    if !tracing::dispatcher::has_been_set() {
        if rotate.rotation_enabled() {
            let max_size_bytes = usize::try_from(rotate.max_size_bytes())
                .context("log_max_size_mb exceeds platform limit")?;
            let append_count = rotate.append_count();
            let file_rotate = FileRotate::new(
                &log_path,
                AppendCount::new(append_count),
                ContentLimit::Bytes(max_size_bytes),
                Compression::None,
                #[cfg(unix)]
                None,
            );
            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file_rotate));
            let registry = tracing_subscriber::registry().with(filter).with(file_layer);
            if log_to_stderr {
                registry.with(fmt::layer()).try_init()?;
            } else {
                registry.try_init()?;
            }
        } else {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .with_context(|| format!("open log file {}", log_path.display()))?;
            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file));
            let registry = tracing_subscriber::registry().with(filter).with(file_layer);
            if log_to_stderr {
                registry.with(fmt::layer()).try_init()?;
            } else {
                registry.try_init()?;
            }
        }
    }

    if rotate.rotation_enabled() {
        tracing::info!(
            log_file = %log_path.display(),
            max_size_mb = rotate.max_size_mb,
            max_files = rotate.max_files,
            "gateway log rotation enabled"
        );
    }

    Ok(log_path)
}

/// Whether a tracing subscriber is active (e.g. embedded gateway running).
pub fn is_tracing_initialized() -> bool {
    tracing::dispatcher::has_been_set()
}

/// Emit one traced line without writing to the log file directly.
pub fn emit_traced_message(level: &str, target: &str, message: &str) {
    if !is_tracing_initialized() {
        return;
    }
    match level {
        "ERROR" => tracing::error!(target = target, "{message}"),
        "WARN" => tracing::warn!(target = target, "{message}"),
        _ => tracing::info!(target = target, "{message}"),
    }
}

/// Append one line to `{data_dir}/logs/gateway.log` without mirroring to tracing.
pub fn append_message_file_only(
    data_dir: &Path,
    level: &str,
    target: &str,
    message: &str,
) -> anyhow::Result<()> {
    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)
        .with_context(|| format!("create logs dir {}", logs_dir.display()))?;

    let log_path = logs_dir.join("gateway.log");
    let ts = OffsetDateTime::now_utc()
        .format(
            &time::macros::format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z"
            ),
        )
        .unwrap_or_else(|_| "unknown".to_string());
    let line = format!("{ts}  {level:<5} {target}: {message}\n");

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open log file {}", log_path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("append log file {}", log_path.display()))?;

    Ok(())
}

/// Append one line to `{data_dir}/logs/gateway.log` in the same format as tracing output.
/// Also mirrors to tracing when the subscriber is initialized.
pub fn append_message(
    data_dir: &Path,
    level: &str,
    target: &str,
    message: &str,
) -> anyhow::Result<()> {
    append_message_file_only(data_dir, level, target, message)?;

    emit_traced_message(level, target, message);

    Ok(())
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

    let end = file_len;
    let skip_first = !matches!(offset, Some(off) if off <= file_len && !reset);
    let lines = read_log_bytes(path, start, end, skip_first)?;

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
    let lines = read_log_bytes(path, start, before, start > 0)?;

    Ok(LogsTail {
        offset: start,
        next_offset: before,
        reset: false,
        lines,
    })
}

fn read_log_bytes(path: &Path, start: u64, end: u64, skip_first: bool) -> anyhow::Result<Vec<LogLine>> {
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
    let mut lines: Vec<LogLine> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| LogLine {
            level: classify_log_level(line),
            text: line.to_string(),
        })
        .collect();

    if skip_first && start > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    // Drop a trailing partial line when the byte range ends mid-line.
    if !buf.is_empty() && !buf.ends_with(b"\n") {
        lines.pop();
    }

    Ok(lines)
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

    #[test]
    fn read_log_tail_initial_load_includes_latest_lines() {
        let path = std::env::temp_dir().join(format!(
            "token-router-log-tail-{}.log",
            std::process::id()
        ));
        let mut content = String::new();
        for i in 0..3000 {
            content.push_str(&format!(
                "2025-01-01T00:00:00.000000Z  INFO token_router: line {i}\n"
            ));
        }
        content.push_str("2025-01-01T00:00:00.000000Z  INFO token_router: LAST_MARKER\n");
        std::fs::write(&path, &content).unwrap();

        let tail = read_log_tail(&path, None, Some(LOG_CHUNK_BYTES)).unwrap();
        let joined = tail
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("LAST_MARKER"),
            "initial tail read must include bytes up to EOF"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_log_tail_incremental_does_not_skip_new_line() {
        let path = std::env::temp_dir().join(format!(
            "token-router-log-incr-{}.log",
            std::process::id()
        ));
        let initial = "2025-01-01T00:00:00.000000Z  INFO token_router: seed\n";
        std::fs::write(&path, initial).unwrap();
        let first = read_log_tail(&path, None, Some(LOG_CHUNK_BYTES)).unwrap();
        assert_eq!(first.lines.len(), 1);

        let appended = format!("{initial}2025-01-01T00:00:01.000000Z  INFO token_router: appended\n");
        std::fs::write(&path, &appended).unwrap();

        let second = read_log_tail(&path, Some(first.next_offset), Some(LOG_CHUNK_BYTES)).unwrap();
        assert_eq!(second.lines.len(), 1);
        assert!(second.lines[0].text.contains("appended"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_log_bytes_drops_trailing_partial_line() {
        let path = std::env::temp_dir().join(format!(
            "token-router-log-partial-{}.log",
            std::process::id()
        ));
        let content = "2025-01-01T00:00:00.000000Z  INFO token_router: complete\n2025-01-01T00:00:01";
        std::fs::write(&path, content).unwrap();

        let lines = read_log_bytes(&path, 0, content.len() as u64, false).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text.contains("complete"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn log_rotate_by_size_archives_files() {
        use std::io::Write;

        use file_rotate::compression::Compression;
        use file_rotate::suffix::AppendCount;
        use file_rotate::{ContentLimit, FileRotate};

        let dir = std::env::temp_dir().join(format!(
            "token-router-log-rotate-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("gateway.log");

        let max_bytes = 512;
        let mut log = FileRotate::new(
            &log_path,
            AppendCount::new(2),
            ContentLimit::Bytes(max_bytes),
            Compression::None,
            #[cfg(unix)]
            None,
        );

        let line = "2025-01-01T00:00:00.000000Z  INFO token_router: rotate test line\n";
        for _ in 0..20 {
            log.write_all(line.as_bytes()).unwrap();
        }

        assert!(log_path.is_file(), "active log must exist");
        assert!(
            dir.join("gateway.log.1").is_file(),
            "first archive must exist after rotation"
        );
        assert!(
            std::fs::metadata(&log_path).unwrap().len() <= max_bytes as u64,
            "active log should be within size limit after rotation"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_rotate_config_disabled_when_size_or_files_zero() {
        assert!(!LogRotateConfig {
            max_size_mb: 0,
            max_files: 5,
        }
        .rotation_enabled());
        assert!(!LogRotateConfig {
            max_size_mb: 10,
            max_files: 1,
        }
        .rotation_enabled());
        assert!(LogRotateConfig {
            max_size_mb: 10,
            max_files: 5,
        }
        .rotation_enabled());
    }
}
