use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, OptionalExtension};

use super::RoutingLogStore;

#[derive(Debug, Clone)]
pub struct RouteCacheRow {
    pub route: String,
    pub model: String,
    pub updated_at: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RouteCacheCleanupStats {
    pub removed_expired: u32,
    pub kept: u32,
}

impl RoutingLogStore {
    pub fn lookup_route_cache(&self, hash_code: &str) -> anyhow::Result<Option<RouteCacheRow>> {
        let conn = self.conn().lock().expect("routing log db mutex");
        conn.query_row(
            "SELECT route, model, updated_at FROM request_route_cache WHERE hash_code = ?1",
            params![hash_code],
            |row| {
                Ok(RouteCacheRow {
                    route: row.get(0)?,
                    model: row.get(1)?,
                    updated_at: row.get::<_, i64>(2)? as u64,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upsert_route_cache(
        &self,
        hash_code: &str,
        route: &str,
        model: &str,
    ) -> anyhow::Result<()> {
        let now = now_unix() as i64;
        let conn = self.conn().lock().expect("routing log db mutex");
        conn.execute(
            "INSERT INTO request_route_cache (hash_code, route, model, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(hash_code) DO UPDATE SET
               route = excluded.route,
               model = excluded.model,
               updated_at = excluded.updated_at",
            params![hash_code, route, model, now],
        )?;
        Ok(())
    }

    pub fn cleanup_route_cache(&self, retention_days: u64) -> anyhow::Result<RouteCacheCleanupStats> {
        if retention_days == 0 {
            return Ok(RouteCacheCleanupStats::default());
        }
        let cutoff = now_unix().saturating_sub(retention_days.saturating_mul(86_400)) as i64;
        let conn = self.conn().lock().expect("routing log db mutex");
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM request_route_cache WHERE updated_at >= ?1",
                params![cutoff],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let removed = conn.execute(
            "DELETE FROM request_route_cache WHERE updated_at < ?1",
            params![cutoff],
        )? as u32;
        if removed > 0 {
            tracing::info!(
                removed_expired = removed,
                kept,
                retention_days,
                "request route cache cleanup"
            );
        }
        Ok(RouteCacheCleanupStats {
            removed_expired: removed,
            kept: kept as u32,
        })
    }

    pub fn spawn_route_cache_cleanup_task(
        self: &Arc<Self>,
        retention_days: u64,
        cleanup_interval_secs: u64,
    ) {
        let store = self.clone();
        let interval_secs = cleanup_interval_secs.max(60);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.cleanup_route_cache(retention_days) {
                    tracing::warn!(error = %e, "request route cache cleanup failed");
                }
            }
        });
    }

    pub(crate) fn conn(&self) -> &std::sync::Mutex<rusqlite::Connection> {
        &self.conn
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("flowy-route-cache-{label}-{}", std::process::id()))
    }

    #[test]
    fn route_cache_upsert_lookup() {
        let dir = temp_dir("upsert");
        std::fs::create_dir_all(&dir).unwrap();
        let store = RoutingLogStore::open(&dir).unwrap();
        store
            .upsert_route_cache("abc", "cloud", "gpt-4o")
            .unwrap();
        let row = store.lookup_route_cache("abc").unwrap().unwrap();
        assert_eq!(row.route, "cloud");
        assert_eq!(row.model, "gpt-4o");
        store
            .upsert_route_cache("abc", "edge", "local")
            .unwrap();
        let row = store.lookup_route_cache("abc").unwrap().unwrap();
        assert_eq!(row.route, "edge");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn route_cache_cleanup_expired() {
        let dir = temp_dir("cleanup");
        std::fs::create_dir_all(&dir).unwrap();
        let store = RoutingLogStore::open(&dir).unwrap();
        store
            .upsert_route_cache("old", "cloud", "gpt-4o")
            .unwrap();
        let cutoff = now_unix().saturating_sub(8 * 86_400) as i64;
        store
            .conn()
            .lock()
            .unwrap()
            .execute(
                "UPDATE request_route_cache SET updated_at = ?1 WHERE hash_code = 'old'",
                rusqlite::params![cutoff],
            )
            .unwrap();
        store.upsert_route_cache("fresh", "edge", "local").unwrap();
        let stats = store.cleanup_route_cache(7).unwrap();
        assert_eq!(stats.removed_expired, 1);
        assert_eq!(stats.kept, 1);
        assert!(store.lookup_route_cache("old").unwrap().is_none());
        assert!(store.lookup_route_cache("fresh").unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn route_cache_upsert_refreshes_ttl() {
        let dir = temp_dir("refresh");
        std::fs::create_dir_all(&dir).unwrap();
        let store = RoutingLogStore::open(&dir).unwrap();
        store
            .upsert_route_cache("k", "cloud", "gpt-4o")
            .unwrap();
        let stale = now_unix().saturating_sub(8 * 86_400);
        store
            .conn()
            .lock()
            .unwrap()
            .execute(
                "UPDATE request_route_cache SET updated_at = ?1 WHERE hash_code = 'k'",
                rusqlite::params![stale as i64],
            )
            .unwrap();
        store.upsert_route_cache("k", "cloud", "gpt-4o").unwrap();
        let fresh = store.lookup_route_cache("k").unwrap().unwrap().updated_at;
        assert!(fresh > stale);
        let stats = store.cleanup_route_cache(7).unwrap();
        assert_eq!(stats.removed_expired, 0);
        assert!(store.lookup_route_cache("k").unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
