//! 配置服务可靠性回归测试（revision 一致性 / CAS / 写锁 / 占位符）。
//!
//! 覆盖：
//! - GET revision 来自磁盘（连续保存不重启不冲突）；
//! - apply If-Match 必需；陈旧 revision → 409；
//! - 两写竞争只能一个成功（串行锁 + CAS）；
//! - rollback 与 GET/apply 的 revision 视图一致；
//! - `redacted:literal` 占位符拒绝回写；
//! - validate normalized_toml 脱敏。

use std::sync::Arc;

use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::Pipeline;
use tr_agent::store::Db;
use tr_control::AdminAuth;
use tr_control::config_service;
use tr_control::state::ControlState;

const BASE: &str = r#"
schema_version = 1

[server]
listen = "127.0.0.1:16621"

[profiles.default]
mode = "cascade"
strength_order = ["local"]

[[candidates]]
id = "a"
tier = "local"
strength = 10
provider = "openai"
base_url = "http://127.0.0.1:11434/v1"
model = "qwen2.5:14b"
window = 32768
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["a"]
"#;

/// 生成改了 judge.mode 的候选文档。
fn doc_with_mode(mode: &str) -> String {
    BASE.replace(
        "candidate_pool = [\"a\"]",
        &format!("candidate_pool = [\"a\"]\n\n[judge]\nmode = \"{mode}\""),
    )
}

/// roundtrip 探针：parse → toml 序列化 → parse 的 digest 必须无损（core normalization 不丢字段）。
#[test]
fn configfile_toml_roundtrip_preserves_digest() {
    for doc in [
        BASE.to_string(),
        doc_with_mode("shadow"),
        doc_with_mode("shadow").replace(
            "model = \"qwen2.5:14b\"",
            "model = \"qwen2.5:14b\"\ncredential = \"sk-live-secret\"",
        ),
    ] {
        let direct = catalog_of(&doc).config_digest;
        let serialized =
            toml::to_string(&ConfigFile::parse(&doc).expect("parse")).expect("serialize");
        let roundtrip = catalog_of(&serialized).config_digest;
        assert_eq!(direct, roundtrip, "roundtrip 丢字段会破坏 CAS 基准: {doc}");
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    state: ControlState,
    catalog: Catalog,
    path: std::path::PathBuf,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, BASE).expect("write config");

    let catalog = ConfigFile::parse(BASE)
        .expect("parse")
        .into_catalog()
        .expect("catalog");
    let db = Db::open_in_memory().expect("db");
    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog.clone()));
    let mut state = ControlState::new(
        pipeline,
        AdminAuth::new(Arc::new(Db::open_in_memory().expect("auth db")), None),
    );
    state = state.with_config_path(path.clone());
    Harness {
        _dir: dir,
        state,
        catalog,
        path,
    }
}

fn catalog_of(text: &str) -> Catalog {
    ConfigFile::parse(text)
        .expect("parse")
        .into_catalog()
        .expect("catalog")
}

/// GET /config 的 revision 必须与磁盘 ConfigFile→into_catalog 的 digest 同源。
#[test]
fn get_revision_matches_disk_digest_not_runtime_catalog() {
    let h = harness();
    let document = config_service::config_document(&h.catalog, &h.state).expect("document");
    let expected = catalog_of(BASE).config_digest;
    assert_eq!(document.revision, expected, "GET revision = 磁盘 digest");

    // 运行 catalog digest 与磁盘一致（本例未漂移）。
    assert_eq!(h.catalog.config_digest, expected);
}

/// 关键缺陷回归：磁盘已更新（其他写者/重启前场景），运行 catalog 还是旧值；
/// GET 必须返回磁盘新 revision，否则前端下次保存必 409。
#[test]
fn get_revision_tracks_disk_after_external_write() {
    let h = harness();
    // 模拟：磁盘被另一个 apply 更新，运行 catalog 未热加载。
    let updated = doc_with_mode("shadow");
    std::fs::write(&h.path, &updated).expect("write updated config");
    let disk_digest = catalog_of(&updated).config_digest;
    assert_ne!(disk_digest, h.catalog.config_digest);

    let document = config_service::config_document(&h.catalog, &h.state).expect("document");
    assert_eq!(
        document.revision, disk_digest,
        "GET revision 必须反映磁盘，而不是运行 catalog 旧值"
    );

    // 用磁盘新 revision 做 CAS 能成功（连续保存不冲突）。
    let outcome = config_service::apply(&h.state, &doc_with_mode("active"), &disk_digest)
        .expect("apply with fresh disk revision");
    assert_eq!(
        outcome.revision,
        catalog_of(&doc_with_mode("active")).config_digest
    );
}

/// 连续两次保存（不重启）：第一次 apply 后 GET 返回新 revision，第二次用它成功。
#[test]
fn consecutive_saves_without_restart() {
    let h = harness();
    let first_base = config_service::disk_revision(&h.state).expect("disk revision");

    let first =
        config_service::apply(&h.state, &doc_with_mode("shadow"), &first_base).expect("first save");
    assert_ne!(first.revision, first_base);

    // 不重启：GET 看到的是磁盘新 revision。
    let catalog = config_service::disk_revision(&h.state).expect("disk revision after save");
    assert_eq!(catalog, first.revision, "磁盘 = apply 返回的 revision");

    // 第二次连续保存用新 base 成功。
    let second = config_service::apply(&h.state, &doc_with_mode("active"), &first.revision)
        .expect("second save");
    assert_ne!(second.revision, first.revision);
}

/// If-Match 缺失（空串）→ invalid_request；陈旧 revision → config_conflict。
#[test]
fn apply_requires_if_match_and_rejects_stale() {
    let h = harness();
    let base = config_service::disk_revision(&h.state).expect("disk revision");

    // 缺失。
    let error = config_service::apply(&h.state, &doc_with_mode("shadow"), "")
        .expect_err("If-Match required");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::InvalidRequest);

    // 陈旧：先写一次，旧 base 立即失效。
    config_service::apply(&h.state, &doc_with_mode("shadow"), &base).expect("first apply");
    let error = config_service::apply(&h.state, &doc_with_mode("active"), &base)
        .expect_err("stale revision must conflict");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::ConfigConflict);

    // 错误值同样 409。
    let error = config_service::apply(&h.state, &doc_with_mode("active"), "deadbeef")
        .expect_err("unknown revision must conflict");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::ConfigConflict);
}

/// 两写竞争：两个线程各持陈旧/新鲜 base 并发 apply，只能一个成功。
#[test]
fn concurrent_writes_only_one_wins() {
    let h = harness();
    let base = Arc::new(config_service::disk_revision(&h.state).expect("disk revision"));

    // 两个写者基于同一 base 竞争。
    let state = Arc::new(h.state.clone());
    let s1 = state.clone();
    let b1 = base.clone();
    let t1 = std::thread::spawn(move || config_service::apply(&s1, &doc_with_mode("shadow"), &b1));
    let s2 = state.clone();
    let b2 = base.clone();
    let t2 = std::thread::spawn(move || config_service::apply(&s2, &doc_with_mode("active"), &b2));
    let r1 = t1.join().expect("t1");
    let r2 = t2.join().expect("t2");

    let winners = [r1.is_ok(), r2.is_ok()].iter().filter(|ok| **ok).count();
    assert_eq!(
        winners, 1,
        "同一 base 的两个并发写必须恰好一个成功: {r1:?} / {r2:?}"
    );

    // 落盘状态确定：是赢家写入的文档。
    let disk = std::fs::read_to_string(&h.path).expect("read disk");
    let digest = catalog_of(&disk).config_digest;
    let winner_revision = match (r1, r2) {
        (Ok(outcome), Err(_)) => outcome.revision,
        (Err(_), Ok(outcome)) => outcome.revision,
        _ => unreachable!(),
    };
    assert_eq!(digest, winner_revision);

    // 输家拿到 409 后可以用最新 revision 重试成功。重试用「输家那份」文档：
    // 若它就是磁盘上的赢家内容，同文档重试 revision 不变，无法区分成功/冲突——
    // 所以按赢家身份选相反的模式，保证重试必然改写磁盘。
    let retry_doc = if disk.contains("mode = \"shadow\"") {
        doc_with_mode("active")
    } else {
        doc_with_mode("shadow")
    };
    let retry =
        config_service::apply(&h.state, &retry_doc, &digest).expect("retry with fresh revision");
    assert_eq!(
        config_service::disk_revision(&h.state).expect("disk"),
        retry.revision
    );
    assert_ne!(retry.revision, digest);
}

/// 回滚：原子写、revision 与 GET/apply 同源、previous_revision 记录回滚前磁盘值。
#[test]
fn rollback_is_consistent_with_disk_view() {
    let h = harness();
    let original = config_service::disk_revision(&h.state).expect("disk revision");

    let applied =
        config_service::apply(&h.state, &doc_with_mode("active"), &original).expect("apply");
    assert_eq!(
        config_service::disk_revision(&h.state).expect("disk"),
        applied.revision
    );
    assert!(applied.backup_path.is_some(), "apply 必须留备份");

    let rolled = config_service::rollback(&h.state).expect("rollback");
    assert_eq!(rolled.revision, original, "回滚后磁盘 = 原始 revision");
    assert_eq!(
        config_service::disk_revision(&h.state).expect("disk"),
        rolled.revision
    );
    assert_eq!(
        rolled.previous_revision.as_deref(),
        Some(applied.revision.as_str()),
        "审计 from = 回滚前的磁盘 revision"
    );

    // 回滚幂等：再次回滚仍还原同一份（最近 apply）备份，revision 不变。
    let rolled_again = config_service::rollback(&h.state).expect("second rollback");
    assert_eq!(rolled_again.revision, original, "rollback 幂等");
    assert_eq!(
        config_service::disk_revision(&h.state).expect("disk"),
        rolled_again.revision
    );
}

/// `redacted:literal` 占位符不能回写；env:/file: 引用与既有 literal 不受影响。
#[test]
fn redacted_placeholder_writeback_rejected() {
    let h = harness();
    let base = config_service::disk_revision(&h.state).expect("disk revision");

    // GET 脱敏视图里 credential 变成 redacted:literal；带占位符回写必须被拒。
    let placeholder_doc = doc_with_mode("shadow").replace(
        "model = \"qwen2.5:14b\"",
        "model = \"qwen2.5:14b\"\ncredential = \"redacted:literal\"",
    );
    let error = config_service::apply(&h.state, &placeholder_doc, &base)
        .expect_err("placeholder must be rejected");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::ConfigInvalid);
    assert!(
        error.message.contains("redacted placeholder"),
        "错误信息要指明占位符: {}",
        error.message
    );
    assert!(
        error.message.contains("candidates.a.credential"),
        "错误信息要指明字段路径: {}",
        error.message
    );
    // 拒绝后磁盘未变。
    assert_eq!(config_service::disk_revision(&h.state).expect("disk"), base);

    // env: 引用可以写。
    let ref_doc = doc_with_mode("shadow").replace(
        "model = \"qwen2.5:14b\"",
        "model = \"qwen2.5:14b\"\ncredential = \"env:TR_KEY\"",
    );
    config_service::apply(&h.state, &ref_doc, &base).expect("env ref accepted");

    // keyring: 引用是内核合法类型，同样放行且脱敏视图原样保留。
    let keyring_doc = doc_with_mode("active").replace(
        "model = \"qwen2.5:14b\"",
        "model = \"qwen2.5:14b\"\ncredential = \"keyring:openai-main\"",
    );
    let fresh = config_service::disk_revision(&h.state).expect("disk revision");
    config_service::apply(&h.state, &keyring_doc, &fresh).expect("keyring ref accepted");
    let document = config_service::config_document(&h.catalog, &h.state).expect("document");
    assert!(
        document
            .secrets
            .iter()
            .any(|secret| secret.reference == "keyring:openai-main"),
        "keyring 引用必须原样出现在 secrets: {:?}",
        document.secrets
    );
    let response = config_service::validate(&h.catalog, &h.state, &keyring_doc).expect("validate");
    assert!(
        response
            .normalized_toml
            .as_deref()
            .unwrap_or_default()
            .contains("credential = \"keyring:openai-main\""),
        "keyring 引用不得被占位符替换"
    );

    // rollback 也拒绝占位符（备份内容含占位符时不还原）。
    let placeholder_backup_doc = doc_with_mode("active").replace(
        "model = \"qwen2.5:14b\"",
        "model = \"qwen2.5:14b\"\ncredential = \"redacted:literal\"",
    );
    // 直接造一个含占位符的备份文件，stamp 取未来微秒值，确保它是"最新备份"
    //（与实现 backup_config 的微秒命名同一量纲，排序兼容）。
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64 + 1_000_000)
        .unwrap_or(0);
    let backup = h.path.with_file_name(format!("config.toml.bak-{stamp}"));
    std::fs::write(&backup, &placeholder_backup_doc).expect("write backup");
    // rollback 对备份内容做同样的占位符守卫：拒绝还原（422 config_invalid，
    // 与 apply 的占位符错误同类——比笼统 internal 更可区分）。
    let error = config_service::rollback(&h.state).expect_err("rollback must reject placeholder");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::ConfigInvalid);
    assert!(error.message.contains("redacted placeholder"));
    // 磁盘未被污染。
    let disk = std::fs::read_to_string(&h.path).expect("disk");
    assert!(!disk.contains("redacted:literal"));
}

/// validate：normalized_toml 脱敏（字面量→占位符，引用保留）；diff 基准是磁盘。
#[test]
fn validate_normalizes_and_redacts() {
    let h = harness();
    let secret_doc = doc_with_mode("shadow").replace(
        "model = \"qwen2.5:14b\"",
        "model = \"qwen2.5:14b\"\ncredential = \"sk-live-secret\"",
    );
    let secret_response =
        config_service::validate(&h.catalog, &h.state, &secret_doc).expect("validate ok");
    assert!(secret_response.valid);
    let normalized = secret_response.normalized_toml.expect("normalized toml");
    assert!(
        !normalized.contains("sk-live-secret"),
        "密钥不得回显: {normalized}"
    );
    assert!(normalized.contains("credential = \"redacted:literal\""));

    // 引用原样保留。
    let ref_doc = doc_with_mode("shadow").replace(
        "model = \"qwen2.5:14b\"",
        "model = \"qwen2.5:14b\"\ncredential = \"file:credentials/a.key\"",
    );
    let ref_response =
        config_service::validate(&h.catalog, &h.state, &ref_doc).expect("validate ok");
    let normalized = ref_response.normalized_toml.expect("normalized toml");
    assert!(
        normalized.contains("credential = \"file:credentials/a.key\""),
        "{normalized}"
    );

    // diff 基准是磁盘文档：磁盘是 BASE，候选把 judge.mode 改成 shadow → 有 diff。
    assert!(
        ref_response.diff.iter().any(|entry| {
            entry.path == "judge"
                && entry.from.as_ref().and_then(serde_json::Value::as_str) == Some("off")
        }),
        "diff from 必须来自磁盘旧值: {:?}",
        ref_response.diff
    );

    // validate 的 revision 是候选 digest（"假如提交"的值），可以提示，但 CAS 基准仍是 GET 磁盘值。
    assert_eq!(
        secret_response.revision.as_deref(),
        Some(catalog_of(&secret_doc).config_digest.as_str())
    );
}

/// 磁盘配置损坏时 GET/apply 返回 internal error（不允许盲写覆盖坏盘）。
#[test]
fn corrupt_disk_config_blocks_apply_and_get() {
    let h = harness();
    std::fs::write(&h.path, "schema_version = 9").expect("corrupt");
    let error = config_service::config_document(&h.catalog, &h.state).expect_err("get fails");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::InternalError);
    let error = config_service::apply(&h.state, &doc_with_mode("shadow"), "whatever")
        .expect_err("apply fails");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::InternalError);
}

/// 自动保存：同一窗口内重复 apply 复用同一份备份（不烧穿回滚环形）。
#[test]
fn apply_reuses_backup_within_window() {
    let h = harness();
    let r0 = config_service::disk_revision(&h.state).expect("disk");
    let first = config_service::apply(&h.state, &doc_with_mode("shadow"), &r0).expect("apply 1");
    let r1 = config_service::disk_revision(&h.state).expect("disk");
    let second = config_service::apply(&h.state, &doc_with_mode("active"), &r1).expect("apply 2");
    assert_eq!(
        first.backup_path, second.backup_path,
        "5 分钟窗口内应复用会话开始前的备份"
    );
    // 复用的备份是会话开始前的配置：回滚一次回到 BASE。
    let rolled = config_service::rollback(&h.state).expect("rollback");
    assert_eq!(rolled.revision, r0);
}

/// 只有 `[server]` 段变化才要求重启；其余段落靠热加载生效。
#[test]
fn restart_required_only_for_server_section() {
    let h = harness();
    let r0 = config_service::disk_revision(&h.state).expect("disk");
    let same = config_service::apply(&h.state, &doc_with_mode("shadow"), &r0).expect("apply");
    assert!(!same.restart_required, "只改 judge 不应要求重启");

    let r1 = config_service::disk_revision(&h.state).expect("disk");
    let changed = doc_with_mode("shadow").replace("127.0.0.1:16621", "127.0.0.1:16622");
    let moved = config_service::apply(&h.state, &changed, &r1).expect("apply listen");
    assert!(moved.restart_required, "listen 变化必须要求重启");
}

/// 热加载：reload_runtime 用磁盘配置替换运行时目录（service 层 apply 不自动 reload）。
#[test]
fn reload_runtime_swaps_catalog() {
    let h = harness();
    let r0 = config_service::disk_revision(&h.state).expect("disk");
    let applied = config_service::apply(&h.state, &doc_with_mode("shadow"), &r0).expect("apply");
    assert_eq!(
        h.state.pipeline.catalog().config_digest,
        r0,
        "apply 只写盘；运行时由调用方 reload"
    );

    let outcome = config_service::reload_runtime(&h.state).expect("reload");
    assert_eq!(outcome.previous_revision, r0);
    assert_eq!(outcome.revision, applied.revision);
    assert_eq!(
        h.state.pipeline.catalog().config_digest,
        applied.revision,
        "reload 后运行时跟随磁盘"
    );
    // 同一份配置重复 reload 幂等。
    let again = config_service::reload_runtime(&h.state).expect("reload again");
    assert_eq!(again.revision, applied.revision);
}
