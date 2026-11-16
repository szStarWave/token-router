//! 存储层测试（P0a-7/8）：迁移幂等、schema 版本门禁、CAS、账本往返。

use std::path::PathBuf;

use tr_agent::store::sessions::SaveOutcome;
use tr_agent::store::{Db, StoreError, health, ledger, sessions, strategy};
use tr_contracts::{
    AttemptRecord, BreakerState, CandidateId, Cost, DispatchSource, FactsCompleteness,
    IngressProtocol, LedgerRecord, OutcomeKind, PlanMode, RequestReason, SessionFacts,
    SessionKeySource, StrategyState, VirtualModelState,
};

/// 临时库：Drop 时先关连接再删文件（Windows 上打开的文件删不掉）。
struct TempDb {
    path: PathBuf,
    db: Option<Db>,
    keep_file: bool,
}

impl TempDb {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "tr-v2-store-{name}-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let db = Db::open(&path).expect("open db");
        Self {
            path,
            db: Some(db),
            keep_file: false,
        }
    }

    fn db(&self) -> &Db {
        self.db.as_ref().expect("db still open")
    }

    /// 关闭连接但保留文件，返回路径（用于“重新打开同一库”的测试）。
    fn into_path(mut self) -> PathBuf {
        drop(self.db.take());
        self.keep_file = true;
        self.path.clone()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        drop(self.db.take());
        if self.keep_file {
            return;
        }
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(self.path.with_extension("sqlite-shm"));
    }
}

fn facts(session_key: &str) -> SessionFacts {
    SessionFacts {
        session_key: session_key.to_string(),
        branch_id: "main".to_string(),
        updated_at: 1_700_000_000_000,
        ..SessionFacts::default()
    }
}

#[test]
fn migrate_is_idempotent_across_reopen() {
    let temp = TempDb::new("migrate");
    assert_eq!(temp.db().schema_version().unwrap(), 1);

    let path = temp.into_path();

    let reopened = Db::open(&path).expect("reopen");
    assert_eq!(reopened.schema_version().unwrap(), 1);
    drop(reopened);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

#[test]
fn newer_schema_refuses_to_start() {
    let temp = TempDb::new("newer");
    let path = temp.into_path();

    {
        let raw = rusqlite::Connection::open(&path).expect("raw open");
        raw.execute(
            "INSERT INTO schema_migrations (version, applied_at, checksum) VALUES (99, 0, 'x')",
            [],
        )
        .expect("insert future version");
    }

    let error = Db::open(&path).expect_err("must refuse newer schema");
    assert!(matches!(
        error,
        StoreError::SchemaTooNew {
            found: 99,
            supported: 1
        }
    ));

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

#[test]
fn session_insert_then_cas_update() {
    let temp = TempDb::new("session");
    let db = temp.db();

    assert!(
        sessions::load(db, &"a".repeat(64), "main")
            .unwrap()
            .is_none()
    );

    let mut record = facts(&"a".repeat(64));
    assert_eq!(
        sessions::save(db, &record).unwrap(),
        SaveOutcome::Saved { version: 1 }
    );

    let loaded = sessions::load(db, &record.session_key, "main")
        .unwrap()
        .expect("session exists");
    assert_eq!(loaded.version, 1);

    // 旧版本号再次保存 → 冲突（另一个写入者先提交了）。
    assert_eq!(sessions::save(db, &record).unwrap(), SaveOutcome::Conflict);

    record.version = 1;
    record.consecutive_failures = 2;
    assert_eq!(
        sessions::save(db, &record).unwrap(),
        SaveOutcome::Saved { version: 2 }
    );
    let loaded = sessions::load(db, &record.session_key, "main")
        .unwrap()
        .expect("session exists");
    assert_eq!(loaded.consecutive_failures, 2);
    assert_eq!(loaded.version, 2);
}

fn ledger_record() -> LedgerRecord {
    LedgerRecord {
        request_id: uuid::Uuid::new_v4(),
        plan_id: "plan-1".to_string(),
        session_key: "c".repeat(64),
        branch_id: "main".to_string(),
        task_id: String::new(),
        session_key_source: SessionKeySource::Content,
        collision_suspect: false,
        ingress_protocol: IngressProtocol::ChatCompletions,
        selected_virtual_model: Some("auto".to_string()),
        selected_thinking_level: Some("off".to_string()),
        request_reason: RequestReason::User,
        plan_mode: PlanMode::Single,
        preference: vec!["qwen".to_string()],
        failover: vec!["qwen".to_string()],
        served_slot: None,
        served_physical_model: Some("qwen2.5:14b".to_string()),
        served_provider: Some("openai_compatible".to_string()),
        served_thinking_level: Some("off".to_string()),
        dispatch_source: DispatchSource::Strategy,
        switches: 0,
        attempts: vec![AttemptRecord {
            candidate_id: CandidateId::from("qwen"),
            duration_ms: 12,
            ..AttemptRecord::default()
        }],
        judge: None,
        input_tokens: Some(10),
        output_tokens: Some(4),
        cache_read_tokens: None,
        cache_write_tokens: None,
        cache_hit_tokens: None,
        answer_cost: Some(Cost {
            micros: 42,
            currency: "USD".to_string(),
        }),
        routing_cost: None,
        cost_equiv: Some(Cost {
            micros: 42,
            currency: "USD".to_string(),
        }),
        facts_completeness: FactsCompleteness::Complete,
        reason_codes: Vec::new(),
        outcome: OutcomeKind::Success,
        http_status: Some(200),
        profile_revision: Some("rev-1".to_string()),
        config_digest: Some("digest".to_string()),
        ttft_ms: Some(5),
        duration_ms: 12,
        created_at: 1_700_000_000_000,
    }
}

#[test]
fn ledger_insert_round_trip() {
    let temp = TempDb::new("ledger");
    let db = temp.db();
    let record = ledger_record();

    ledger::insert(db, &record).unwrap();
    assert_eq!(ledger::count(db).unwrap(), 1);
    assert_eq!(
        ledger::cost_equiv_of(db, &record.request_id.to_string())
            .unwrap()
            .unwrap()
            .micros,
        42
    );

    // 失败请求也要落账（§8.4）。
    let mut failed = ledger_record();
    failed.outcome = OutcomeKind::UpstreamError;
    failed.served_physical_model = None;
    failed.http_status = Some(502);
    ledger::insert(db, &failed).unwrap();
    assert_eq!(ledger::count(db).unwrap(), 2);
}

#[test]
fn breaker_round_trip() {
    let temp = TempDb::new("breaker");
    let db = temp.db();

    health::upsert(
        db,
        "qwen",
        &BreakerState {
            consecutive_failures: 3,
            opened_count: 1,
            open_until: Some(1_700_000_060),
            last_error_class: Some(tr_contracts::ErrorClass::Overloaded),
            updated_at: 1_700_000_000,
            ..BreakerState::default()
        },
    )
    .unwrap();

    let loaded = health::load(db).unwrap();
    let state = loaded.state_of("qwen").expect("breaker");
    assert_eq!(state.consecutive_failures, 3);
    assert_eq!(
        state.last_error_class,
        Some(tr_contracts::ErrorClass::Overloaded)
    );
    assert!(loaded.is_open("qwen", 1_700_000_030));
}

#[test]
fn strategy_and_virtual_model_state_use_cas() {
    let temp = TempDb::new("strategy");
    let db = temp.db();

    let state = StrategyState {
        branch_id: "main".to_string(),
        task_id: "task-1".to_string(),
        virtual_model_id: "auto".to_string(),
        strategy_id: "escalation".to_string(),
        state_version: 1,
        blob: serde_json::json!({"streak": 1}),
        updated_at: 1_700_000_000,
        version: 0,
    };
    assert_eq!(
        strategy::save_strategy_state(db, &"d".repeat(64), &state).unwrap(),
        SaveOutcome::Saved { version: 1 }
    );
    assert_eq!(
        strategy::save_strategy_state(db, &"d".repeat(64), &state).unwrap(),
        SaveOutcome::Conflict
    );

    let loaded =
        strategy::load_strategy_states(db, &"d".repeat(64), "main", "task-1", "auto").unwrap();
    assert_eq!(loaded["escalation"].blob["streak"], 1);

    let vm_state = VirtualModelState {
        session_key: "d".repeat(64),
        branch_id: "main".to_string(),
        virtual_model_id: "auto".to_string(),
        state: serde_json::json!({"model": "deepseek"}),
        version: 0,
    };
    assert_eq!(
        strategy::save_virtual_model_state(db, &vm_state).unwrap(),
        SaveOutcome::Saved { version: 1 }
    );
    let loaded = strategy::load_virtual_model_state(db, &"d".repeat(64), "main", "auto")
        .unwrap()
        .expect("vm state");
    assert_eq!(loaded.state["model"], "deepseek");
}
