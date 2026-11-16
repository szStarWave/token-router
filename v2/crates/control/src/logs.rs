//! 日志查询（P4-017）：ring buffer 读取 + 过滤 + 分页。

use tr_contracts::{LogEntry, LogsResponse};

/// ring 缓冲（含 seq 单调递增与过滤分页）。
#[derive(Clone)]
pub struct LogTail {
    inner: crate::state::LogRing,
}

impl LogTail {
    pub fn new(ring: crate::state::LogRing) -> Self {
        Self { inner: ring }
    }

    pub fn push(&self, level: &str, target: &str, message: &str) {
        self.inner.push(level, target, message);
    }

    /// 查询：cursor 之后的最多 `limit` 条（seq 升序）；返回 next_cursor。
    pub fn query(
        &self,
        cursor: i64,
        limit: u32,
        filter: impl Fn(&LogEntry) -> bool,
    ) -> LogsResponse {
        let entries = self.snapshot();
        let items: Vec<LogEntry> = entries
            .into_iter()
            .filter(|entry| entry.seq > cursor && filter(entry))
            .take(limit as usize)
            .collect();
        let next_cursor = items.last().map(|entry| entry.seq);
        LogsResponse { items, next_cursor }
    }

    fn snapshot(&self) -> Vec<LogEntry> {
        self.inner.snapshot()
    }
}
