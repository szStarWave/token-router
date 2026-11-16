//! 控制面运行时状态：pipeline、鉴权、配置路径、日志环形缓冲与关闭信号。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tr_agent::pipeline::Pipeline;

use crate::auth::AdminAuth;
use crate::error::AdminApiError;
use crate::logs::LogTail;

/// 控制面状态。
#[derive(Clone)]
pub struct ControlState {
    pub pipeline: Arc<Pipeline>,
    pub auth: AdminAuth,
    /// 启动时的配置文件路径（`config/apply` 只允许写这一个文件）。
    pub config_path: Option<std::path::PathBuf>,
    /// 数据目录（doctor/备份使用）。
    pub data_dir: Option<std::path::PathBuf>,
    /// 服务端内存日志环形缓冲。
    pub logs: Option<LogTail>,
    /// 进程启动时间（Unix 毫秒）。
    pub started_at: i64,
    /// 优雅关闭信号（supervisor/CLI 触发）。
    shutdown: Arc<AtomicBool>,
}

impl ControlState {
    pub fn new(pipeline: Arc<Pipeline>, auth: AdminAuth) -> Self {
        Self {
            pipeline,
            auth,
            config_path: None,
            data_dir: None,
            logs: None,
            started_at: now_millis(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_config_path(mut self, path: std::path::PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            self.data_dir = Some(parent.to_path_buf());
        }
        self.config_path = Some(path);
        self
    }

    pub fn with_logs(mut self, logs: LogTail) -> Self {
        self.logs = Some(logs);
        self
    }

    /// 日志查询（logs.rs 回调）。
    pub fn log_tail(
        &self,
        cursor: i64,
        limit: u32,
        filter: impl Fn(&tr_contracts::LogEntry) -> bool,
    ) -> Result<tr_contracts::LogsResponse, AdminApiError> {
        let Some(logs) = &self.logs else {
            return Ok(tr_contracts::LogsResponse {
                items: Vec::new(),
                next_cursor: None,
            });
        };
        Ok(logs.query(cursor, limit, filter))
    }

    /// 请求优雅关闭。
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// 是否已请求关闭（main.rs 轮询）。
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// 记录一条日志（server 侧写入 ring）。
    pub fn record_log(&self, level: &str, target: &str, message: &str) {
        if let Some(logs) = &self.logs {
            logs.push(level, target, message);
        }
    }
}

/// 最近 2000 行日志环形缓冲（契约 §4.10）。
#[derive(Clone)]
pub struct LogRing {
    entries: Arc<Mutex<Vec<tr_contracts::LogEntry>>>,
    next_seq: Arc<std::sync::atomic::AtomicI64>,
}

impl LogRing {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            next_seq: Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    pub fn push(&self, level: &str, target: &str, message: &str) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst) + 1;
        entries.push(tr_contracts::LogEntry {
            seq,
            at: now_millis(),
            level: level.to_string(),
            target: target.to_string(),
            // 写入前脱敏：Authorization/x-api-key/api_key/token 值替换。
            message: sanitize(message),
        });
        if entries.len() > RING_CAPACITY {
            let overflow = entries.len() - RING_CAPACITY;
            entries.drain(..overflow);
        }
    }

    /// 快照（seq 升序）。
    pub fn snapshot(&self) -> Vec<tr_contracts::LogEntry> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new()
    }
}

/// ring 容量：最近 2000 行。
pub const RING_CAPACITY: usize = 2000;

/// 脱敏：token/api_key/authorization 字段的值替换为 `***`。
fn sanitize(message: &str) -> String {
    let mut out = message.to_string();
    for key in [
        "Authorization",
        "authorization",
        "x-api-key",
        "api_key",
        "apiKey",
        "token",
    ] {
        if let Some(index) = out.find(key) {
            let after = &out[index + key.len()..];
            if let Some(colon) = after.find(':') {
                let value_start = index + key.len() + colon + 1;
                let rest = &out[value_start..];
                let value_end = rest.find([',', '}', '"']).unwrap_or(rest.len());
                out.replace_range(value_start..value_start + value_end, "***");
            }
        }
    }
    out
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
