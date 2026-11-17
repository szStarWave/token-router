//! SQLite 存储（§8.3、§18.4.1）。
//!
//! 拓扑：**单写连接**（`Mutex` 串行）+ 4 个读连接；内存库退化为单连接。
//! 写事务必须短：决策/执行/账本提交一次事务，任何网络等待都不得在事务内。
//!
//! 仓储函数全部是显式传参的纯 DB 操作，不做策略判断；`agent::pipeline` 负责串联。

pub mod admin_query;
pub mod approvals;
pub mod config_audit;
pub mod health;
pub mod judge;
pub mod ledger;
pub mod limits;
mod migrate;
pub mod protocol;
pub mod providers;
pub mod quota;
pub mod sessions;
pub mod stats;
pub mod strategy;
pub mod tokens;
pub mod usage;

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

/// 读连接数量（§18.4.1）。
const READER_CONNECTIONS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("migration {version} checksum mismatch")]
    ChecksumMismatch { version: i64 },
    #[error("database schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("invalid query: {0}")]
    Invalid(String),
}

/// 数据库句柄。
#[derive(Debug)]
pub struct Db {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
}

impl Db {
    /// 打开（或创建）文件库，应用 pragmas 与迁移。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let mut writer = open_connection(path)?;
        migrate::migrate(&mut writer)?;

        let mut readers = Vec::with_capacity(READER_CONNECTIONS);
        for _ in 0..READER_CONNECTIONS {
            readers.push(Mutex::new(open_connection(path)?));
        }

        Ok(Self {
            writer: Mutex::new(writer),
            readers,
            next_reader: AtomicUsize::new(0),
        })
    }

    /// 内存库，仅用于测试。读也走写连接（`:memory:` 的多个连接是不同的库）。
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut writer = Connection::open_in_memory()?;
        apply_pragmas(&writer)?;
        migrate::migrate(&mut writer)?;
        Ok(Self {
            writer: Mutex::new(writer),
            readers: Vec::new(),
            next_reader: AtomicUsize::new(0),
        })
    }

    /// 当前 schema 版本（`doctor` 与迁移测试使用）。
    pub fn schema_version(&self) -> Result<i64, StoreError> {
        let connection = self.read();
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        Ok(version)
    }

    pub(crate) fn write(&self) -> MutexGuard<'_, Connection> {
        self.writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn read(&self) -> MutexGuard<'_, Connection> {
        if self.readers.is_empty() {
            return self.write();
        }
        let index = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        self.readers[index]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn open_connection(path: &Path) -> Result<Connection, StoreError> {
    let connection = Connection::open(path)?;
    apply_pragmas(&connection)?;
    Ok(connection)
}

fn apply_pragmas(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA temp_store = MEMORY;",
    )?;
    Ok(())
}

/// 一次请求的持久化提交：会话事实（CAS）+ 熔断状态 + 账本 + route state，同事务。
///
/// 返回会话 CAS 结果；冲突时调用方仍需保留账本（账本已在同一事务里写入），
/// 并补 `STATE_CAS_CONFLICT` 理由码。
pub fn commit(
    db: &Db,
    facts: Option<&tr_contracts::SessionFacts>,
    breakers: &[(tr_contracts::CandidateId, tr_contracts::BreakerState)],
    ledger: &tr_contracts::LedgerRecord,
    route_state: Option<&tr_contracts::VirtualModelState>,
) -> Result<Option<sessions::SaveOutcome>, StoreError> {
    let mut connection = db.write();
    let transaction = connection.transaction()?;

    let session = match facts {
        Some(facts) => Some(sessions::save_with(&transaction, facts)?),
        None => None,
    };
    for (candidate_id, state) in breakers {
        health::upsert_with(&transaction, candidate_id, state)?;
    }
    ledger::insert_with(&transaction, ledger)?;
    // route state 只做审计/供 route 插件读取；CAS 冲突说明另一请求写了更新版本，
    // 不覆盖、也不让请求失败（§7：它不是 sticky，丢了不影响本轮决策）。
    if let Some(state) = route_state {
        let _ = strategy::save_virtual_model_state_with(&transaction, state)?;
    }

    transaction.commit()?;
    Ok(session)
}
