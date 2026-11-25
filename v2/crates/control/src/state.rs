//! 控制面运行时状态：pipeline、鉴权、配置路径、日志环形缓冲与关闭信号。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tr_agent::pipeline::Pipeline;

use crate::auth::AdminAuth;
use crate::error::AdminApiError;
use crate::events::EventBus;
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
    /// SSE 事件总线（P4-081）。
    pub events: Arc<EventBus>,
    /// 进程启动时间（Unix 毫秒）。
    pub started_at: i64,
    /// 进程启动时生效的 `[server]` 段（热加载不改监听/鉴权；用于 restart 判定）。
    running_server: Option<Arc<tr_agent::config::ServerConfig>>,
    /// 优雅关闭信号（supervisor/CLI 触发）。
    shutdown: Arc<AtomicBool>,
    /// 配置写锁（可靠性修复）：apply / rollback / provider compile 的
    /// 「读盘 → 改 → 写」全程必须持锁，防止管理面自身两个写者交错。
    /// 只序列化本服务内的写者；外部进程手改 config.toml 无 OS 级锁，
    /// 由 CAS（If-Match 对当前磁盘 revision）兜底。
    config_write_lock: Arc<Mutex<()>>,
    /// 账户余额查询缓存（account_id → (fetched_at, info)），进程内 60s TTL。
    usage_cache:
        Arc<Mutex<std::collections::HashMap<String, (i64, tr_contracts::AccountUsageInfo)>>>,
}

impl ControlState {
    pub fn new(pipeline: Arc<Pipeline>, auth: AdminAuth) -> Self {
        Self {
            pipeline,
            auth,
            config_path: None,
            data_dir: None,
            logs: None,
            events: Arc::new(EventBus::new()),
            started_at: now_millis(),
            shutdown: Arc::new(AtomicBool::new(false)),
            running_server: None,
            config_write_lock: Arc::new(Mutex::new(())),
            usage_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// 账户余额缓存（克隆 Arc，调用方只在读写瞬间持锁，不跨 await）。
    pub(crate) fn usage_cache(
        &self,
    ) -> Arc<Mutex<std::collections::HashMap<String, (i64, tr_contracts::AccountUsageInfo)>>> {
        self.usage_cache.clone()
    }

    /// 配置写锁的守卫。持锁期间再取锁会阻塞（不许嵌套：compile 持锁后
    /// 只能走 `config_service::apply_locked` 这类不加锁的内部 helper）。
    pub fn config_write_lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.config_write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn with_config_path(mut self, path: std::path::PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            self.data_dir = Some(parent.to_path_buf());
        }
        self.config_path = Some(path);
        self
    }

    /// 记录进程启动时生效的 `[server]` 段：热加载不会改变监听/鉴权，
    /// 用它判断磁盘改动是否需要重启（`None` = 未知，视为不需要）。
    pub fn with_running_server(mut self, server: tr_agent::config::ServerConfig) -> Self {
        self.running_server = Some(std::sync::Arc::new(server));
        self
    }

    /// 启动时生效的 `[server]` 段（测试/CLI 未设置时为 `None`）。
    pub fn running_server(&self) -> Option<tr_agent::config::ServerConfig> {
        self.running_server.as_deref().cloned()
    }

    pub fn with_logs(mut self, logs: LogTail) -> Self {
        self.logs = Some(logs);
        self
    }

    /// 启动 ledger/熔断 → 事件同步任务（P4-081）。
    ///
    /// 每 2s 检查一次：
    /// - 账本行数增长 → 每个 ID 区间发一条 `request.completed`（元数据摘要）；
    /// - 熔断从 closed → open → `breaker.opened`。
    ///
    /// 事件只含 ID 与元数据，不含 prompt/正文（§4.9）。
    pub fn spawn_event_sync(&self) {
        let pipeline = self.pipeline.clone();
        let bus = self.events.clone();
        let shutdown_flag = {
            // 借 shutdown AtomicBool 的克隆：ControlState 内部字段私有，
            // 用一个独立标志。
            std::sync::Arc::new(AtomicBool::new(false))
        };
        let _ = shutdown_flag;
        tokio::spawn(async move {
            let mut last_count: i64 = tr_agent::store::ledger::count(pipeline.db()).unwrap_or(0);
            let mut last_open: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                // 账本增长：发 request.completed（新行的 id 区间）。
                let current = tr_agent::store::ledger::count(pipeline.db()).unwrap_or(last_count);
                if current > last_count {
                    if let Ok(rows) = tr_agent::store::admin_query::list_requests(
                        pipeline.db(),
                        &tr_agent::store::admin_query::LedgerQuery {
                            limit: (current - last_count).clamp(1, 20) as u32,
                            ..Default::default()
                        },
                    ) {
                        for row in rows {
                            bus.publish(
                                crate::events::EventKind::RequestCompleted,
                                serde_json::json!({
                                    "request_id": row.request_id,
                                    "outcome": row.outcome,
                                    "model": row.dispatch_model,
                                    "cost_micros": row.cost_micros.answer + row.cost_micros.routing,
                                }),
                            );
                        }
                    }
                    last_count = current;
                }
                // 熔断：新打开的候选 → breaker.opened。
                if let Ok(health) = tr_agent::store::health::load(pipeline.db()) {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_secs())
                        .unwrap_or(0);
                    let open_now: std::collections::BTreeSet<String> = health
                        .candidates
                        .iter()
                        .filter(|(_, state)| state.is_open(now))
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in open_now.difference(&last_open) {
                        bus.publish(
                            crate::events::EventKind::BreakerOpened,
                            serde_json::json!({"candidate_id": id}),
                        );
                    }
                    last_open = open_now;
                }
            }
        });
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
