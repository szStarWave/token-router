//! 配置服务（P4-031/032/033）：文档脱敏视图、validate、apply（CAS + 原子写 + 备份）、rollback。
//!
//! 硬规则（技术方案 §4.5/§4.7）：
//! - 不返回密钥值：`credential` 只返回 `env:`/`file:` 引用；字面量一律 `redacted:literal`；
//! - `revision` = 规范化 JSON 的 SHA-256（与账本 `config_digest` 同源）；
//! - `apply` 必须 `If-Match`；不匹配 → `409 config_conflict`；
//! - 原子写：临时文件 → rename；旧文件保留 `config.toml.bak-<unix>`（最多 10 份）；
//! - 服务端不自我重启：apply 只写文件并返回 `restart_required: true`。
//!
//! 可靠性修复（revision 一致性）：
//! - GET / validate / apply 的 revision 基准一律是**当前磁盘配置**
//!   （磁盘 `ConfigFile` → `into_catalog().config_digest`，与 GET 文档同源）；
//! - 运行时 `catalog.config_digest` 继续表示**运行中**配置（不热加载），
//!   只出现在 `GET /status`；不作为编辑/CAS 基准；
//! - apply / rollback / provider compile 共用 [`crate::state::ControlState::config_write_lock`]，
//!   管理面内部写者串行；外部进程手改文件由 CAS 兜底（无 OS 级文件锁）。

use tr_agent::config::{Catalog, ConfigFile};
use tr_contracts::{ConfigDiffEntry, ConfigDocument, ConfigIssue, SecretRef, ValidateResponse};

use crate::error::{AdminApiError, ApiErrorType};
use crate::state::ControlState;

/// 备份保留上限。
const MAX_BACKUPS: usize = 20;

/// 备份合并窗口：自动保存下不每次写盘都留备份（会烧穿回滚环形），
/// 距最近一份备份不足该窗口就复用它（它代表本次编辑会话之前的状态）。
const BACKUP_MIN_INTERVAL_MICROS: u128 = 5 * 60 * 1_000_000;

/// 读取当前磁盘配置并解析。GET/validate/apply/rollback 的统一基准。
pub fn read_config_file(state: &ControlState) -> Result<ConfigFile, AdminApiError> {
    let path = config_path(state)?;
    let text = std::fs::read_to_string(path)
        .map_err(|error| AdminApiError::internal(format!("read config: {error}")))?;
    ConfigFile::parse(&text)
        .map_err(|error| AdminApiError::internal(format!("parse config: {error}")))
}

/// 当前磁盘配置的 revision（与 `into_catalog` 的 digest 同源）。
/// 磁盘损坏时返回 internal error（此时不应允许 apply 盲写）。
pub fn disk_revision(state: &ControlState) -> Result<String, AdminApiError> {
    let file = read_config_file(state)?;
    let catalog = into_catalog_for_state(state, &file).map_err(|error| {
        AdminApiError::internal(format!("current disk config invalid: {error}"))
    })?;
    Ok(catalog.config_digest)
}

fn config_path(state: &ControlState) -> Result<&std::path::PathBuf, AdminApiError> {
    state
        .config_path
        .as_ref()
        .ok_or_else(|| AdminApiError::internal("config path not set"))
}

/// 配置文件所在目录：判官 `file:` 相对引用的解析基准。
fn config_base_dir(state: &ControlState) -> Option<std::path::PathBuf> {
    if let Some(path) = &state.config_path {
        if let Ok(absolute) = std::fs::canonicalize(path) {
            if let Some(parent) = absolute.parent() {
                return Some(parent.to_path_buf());
            }
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                return Some(parent.to_path_buf());
            }
        }
    }
    state.data_dir.clone()
}

/// 先按配置文件目录展开判官凭据的相对 `file:` 引用，再投影运行时目录。
///
/// 磁盘文档保持相对引用不变（脱敏视图、diff、写盘都用原文档），
/// 只有内存里的凭据解析依赖这个基准。
fn into_catalog_for_state(
    state: &ControlState,
    file: &ConfigFile,
) -> Result<Catalog, tr_agent::config::ConfigError> {
    into_catalog_with_base(file, config_base_dir(state).as_deref())
}

fn into_catalog_with_base(
    file: &ConfigFile,
    base: Option<&std::path::Path>,
) -> Result<Catalog, tr_agent::config::ConfigError> {
    let mut resolved = file.clone();
    if let Some(base) = base {
        resolved.resolve_file_credentials(base);
    }
    resolved.into_catalog()
}

/// `GET /v2/admin/config`：脱敏文档视图。
pub fn config_document(
    _catalog: &Catalog,
    state: &ControlState,
) -> Result<ConfigDocument, AdminApiError> {
    // `_catalog`（运行时目录）不再参与：编辑基准是磁盘文档（可靠性修复）。
    // 从配置文件重新读原文（pipeline 里的是运行时 Catalog，已经过归一）。
    let file = read_config_file(state)?;
    let revision = into_catalog_for_state(state, &file)
        .map_err(|error| AdminApiError::internal(format!("current disk config invalid: {error}")))?
        .config_digest;
    let mut document_value =
        serde_json::to_value(&file).map_err(|error| AdminApiError::internal(error.to_string()))?;
    let secrets = redact_document(&mut document_value);
    // revision 取当前磁盘文档（编辑/CAS 基准），不是运行中 catalog 的旧 digest。
    Ok(ConfigDocument {
        schema_version: file.schema_version,
        revision,
        editable: true,
        document: document_value,
        secrets,
        warnings: Vec::new(),
    })
}

/// `POST /v2/admin/config/validate`：纯校验（不写盘）。
///
/// 注意：`normalized_toml` 在回传前会做与 GET 文档一致的脱敏
/// （credential/admin_token/inbound_key 字面量替换为 `redacted:literal`，
/// `env:`/`file:` 引用原样保留），否则提交草稿里的密钥会被原样回显给 WebView。
pub fn validate(
    _catalog: &Catalog,
    state: &ControlState,
    document_toml: &str,
) -> Result<ValidateResponse, AdminApiError> {
    // `_catalog`（运行时目录）不再参与：diff 基准 = 当前磁盘配置
    // （不是运行 catalog；两者在 apply 后会漂移）。
    // 磁盘损坏时显式报错，而不是静默产出空 diff（会误导"无变更"）。
    let disk_file = read_config_file(state)?;
    match ConfigFile::parse(document_toml) {
        Ok(file) => match into_catalog_for_state(state, &file) {
            Ok(new_catalog) => {
                // normalized_toml 已在 redact_toml 内脱敏；JSON 视图仅用于结构化展示
                //（也脱敏，避免任何密钥值流回 WebView）。
                let mut document_value = serde_json::to_value(&file)
                    .map_err(|error| AdminApiError::internal(error.to_string()))?;
                let _ = redact_document(&mut document_value);
                let diff = diff_documents_disk(&disk_file, &file);
                Ok(ValidateResponse {
                    valid: true,
                    revision: Some(new_catalog.config_digest),
                    normalized_toml: Some(redact_toml(&file)),
                    issues: Vec::new(),
                    diff,
                })
            }
            Err(error) => Ok(ValidateResponse {
                valid: false,
                revision: None,
                normalized_toml: None,
                issues: vec![issue_from_config_error(&error)],
                diff: Vec::new(),
            }),
        },
        Err(error) => Ok(ValidateResponse {
            valid: false,
            revision: None,
            normalized_toml: None,
            issues: vec![ConfigIssue {
                path: String::new(),
                code: "parse_error".to_string(),
                message: error.to_string(),
            }],
            diff: Vec::new(),
        }),
    }
}

/// apply 的内部返回（handler 再转契约 DTO）。
#[derive(Debug)]
pub struct ApplyOutcome {
    pub revision: String,
    pub previous_revision: Option<String>,
    pub backup_path: Option<String>,
    pub diff: Vec<ConfigDiffEntry>,
    /// `[server]` 段是否变化（监听/鉴权在进程启动时固化 → 必须重启）。
    pub restart_required: bool,
}

/// 热加载结果：运行时目录已被替换（不影响在途请求）。
#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    pub revision: String,
    pub previous_revision: String,
    /// 磁盘 `[server]` 段与进程启动时不同 → 需要重启才能生效。
    pub restart_required: bool,
}

/// `POST /v2/admin/config/apply`：CAS（If-Match 必需）+ 原子写 + 备份。
///
/// CAS 基准是**当前磁盘配置**的 revision（与 GET /config 一致）；
/// `expected_revision` 缺失 → `invalid_request`（不带 If-Match 不允许写盘）。
/// 调用方必须已持有 [`ControlState::config_write_lock`]（公开 API 入口加锁，
/// provider compile 等读改写流程走 [`apply_locked`]）。
pub fn apply(
    state: &ControlState,
    document_toml: &str,
    expected_revision: &str,
) -> Result<ApplyOutcome, AdminApiError> {
    let guard = state.config_write_lock();
    apply_locked(state, document_toml, expected_revision, &guard)
}

/// 写盘 + 热加载的原子组合：持锁完成「apply → reload」，中间不给其他写者插队，
/// 避免"写成功但 reload 了别人刚写的版本"。
pub fn apply_and_reload(
    state: &ControlState,
    document_toml: &str,
    expected_revision: &str,
) -> Result<(ApplyOutcome, Result<ReloadOutcome, AdminApiError>), AdminApiError> {
    let guard = state.config_write_lock();
    let outcome = apply_locked(state, document_toml, expected_revision, &guard)?;
    let reload = reload_locked(state, &guard);
    Ok((outcome, reload))
}

/// [`apply`] 的无锁版本：调用方必须已持有 `config_write_lock` 守卫
/// （provider compile 的读改写全程持锁，不能嵌套取锁）。
pub fn apply_locked(
    state: &ControlState,
    document_toml: &str,
    expected_revision: &str,
    _guard: &std::sync::MutexGuard<'_, ()>,
) -> Result<ApplyOutcome, AdminApiError> {
    if expected_revision.trim().is_empty() {
        return Err(AdminApiError::invalid_request(
            "If-Match (expected_revision) is required for config apply",
        ));
    }

    let path = config_path(state)?.clone();

    // CAS：If-Match 必须匹配当前磁盘 revision（GET /config 同源）。
    let current = disk_revision(state)?;
    if expected_revision != current {
        return Err(AdminApiError::new(
            ApiErrorType::ConfigConflict,
            format!(
                "revision mismatch: expected {expected_revision}, current {current} \
                 (refetch GET /v2/admin/config for the latest revision)"
            ),
        ));
    }

    // 解析 + 校验（apply 返回 422；validate 返回 200+valid:false）。
    let file = ConfigFile::parse(document_toml).map_err(|error| {
        AdminApiError::with_issues(
            ApiErrorType::ConfigInvalid,
            format!("parse error: {error}"),
            vec![ConfigIssue {
                path: String::new(),
                code: "parse_error".to_string(),
                message: error.to_string(),
            }],
        )
    })?;
    reject_redacted_placeholders(&file)?;
    let new_catalog = into_catalog_for_state(state, &file).map_err(|error| {
        AdminApiError::with_issues(
            ApiErrorType::ConfigInvalid,
            error.to_string(),
            vec![issue_from_config_error(&error)],
        )
    })?;

    // diff/审计基准：写之前的磁盘配置（不是运行 catalog）。
    let current_file = read_config_file(state)?;
    let diff = diff_documents_disk(&current_file, &file);
    let previous_revision = current;
    // `[server]` 在进程启动时固化（监听/鉴权/TLS），只有它变化才需要重启；
    // 其余段落由调用方在写盘后热加载。
    let restart_required = current_file.server != file.server;

    // 写入前备份。
    let backup_path = backup_config(&path)?;

    // 原子写：同目录临时文件 → rename。
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let temp = path.with_extension(format!("toml.tmp-{stamp}"));
    std::fs::write(&temp, document_toml.as_bytes())
        .map_err(|error| AdminApiError::internal(format!("write temp: {error}")))?;
    std::fs::rename(&temp, &path)
        .map_err(|error| AdminApiError::internal(format!("rename config: {error}")))?;

    // 写后读回校验（防半写）。
    if let Err(error) = ConfigFile::parse(
        &std::fs::read_to_string(&path)
            .map_err(|error| AdminApiError::internal(error.to_string()))?,
    ) {
        // 读回失败：回滚。
        if let Some(backup) = backup_path.as_deref() {
            let _ = std::fs::copy(backup, &path);
        }
        return Err(AdminApiError::internal(format!(
            "post-write verification failed, rolled back: {error}"
        )));
    }

    Ok(ApplyOutcome {
        revision: new_catalog.config_digest,
        previous_revision: Some(previous_revision),
        backup_path,
        diff,
        restart_required,
    })
}

/// `POST /v2/admin/config/rollback`：还原最近一份备份。
pub fn rollback(state: &ControlState) -> Result<ApplyOutcome, AdminApiError> {
    let _guard = state.config_write_lock();
    rollback_locked(state)
}

/// 回滚 + 热加载的原子组合（同 [`apply_and_reload`]）。
pub fn rollback_and_reload(
    state: &ControlState,
) -> Result<(ApplyOutcome, Result<ReloadOutcome, AdminApiError>), AdminApiError> {
    let guard = state.config_write_lock();
    let outcome = rollback_locked(state)?;
    let reload = reload_locked(state, &guard);
    Ok((outcome, reload))
}

/// [`rollback`] 的无锁版本：调用方必须已持有 `config_write_lock` 守卫。
pub fn rollback_locked(state: &ControlState) -> Result<ApplyOutcome, AdminApiError> {
    let path = config_path(state)?.clone();
    let parent = path
        .parent()
        .ok_or_else(|| AdminApiError::internal("config path has no parent"))?
        .to_path_buf();
    let base_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.toml".to_string());

    // 找最新的 .bak-<unix>（按数字 stamp 排序，不依赖 read_dir 顺序）。
    let mut backups: Vec<(u64, std::path::PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&parent) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(stamp) = name
                .strip_prefix(&format!("{base_name}.bak-"))
                .and_then(|suffix| suffix.parse::<u64>().ok())
            {
                backups.push((stamp, entry.path()));
            }
        }
    }
    backups.sort_by_key(|(stamp, _)| *stamp);
    let Some((_, latest)) = backups.pop() else {
        return Err(AdminApiError::not_found("no backup to roll back to"));
    };

    let text = std::fs::read_to_string(&latest)
        .map_err(|error| AdminApiError::internal(format!("read backup: {error}")))?;
    let file = ConfigFile::parse(&text)
        .map_err(|error| AdminApiError::internal(format!("backup is invalid: {error}")))?;
    reject_redacted_placeholders(&file)?;
    // 回滚同样只对 `[server]` 段敏感：其余段落热加载即可。
    let restart_required = read_config_file(state)
        .map(|current| current.server != file.server)
        .unwrap_or(true);
    let new_catalog = into_catalog_for_state(state, &file)
        .map_err(|error| AdminApiError::internal(format!("backup is invalid: {error}")))?;

    // 审计基准：回滚前磁盘上的 revision。
    let previous_revision = disk_revision(state).ok();

    // 原子写：临时文件 → rename（与 apply 同一写路径）。
    // 契约保持：rollback 幂等还原「最近一份 apply 备份」，自身不再级联备份。
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let temp = path.with_extension(format!("toml.tmp-{stamp}"));
    std::fs::write(&temp, text.as_bytes())
        .map_err(|error| AdminApiError::internal(format!("write temp: {error}")))?;
    std::fs::rename(&temp, &path)
        .map_err(|error| AdminApiError::internal(format!("rename config: {error}")))?;

    Ok(ApplyOutcome {
        revision: new_catalog.config_digest,
        previous_revision,
        backup_path: Some(latest.to_string_lossy().to_string()),
        diff: Vec::new(),
        restart_required,
    })
}

/// 重读磁盘配置并热替换运行时目录（不写盘）。
///
/// 供 `POST /v2/admin/config/reload` 与 apply/rollback/compile 的内部调用使用；
/// 与配置写者共用 `config_write_lock`，避免读到半写文件。
pub fn reload_runtime(state: &ControlState) -> Result<ReloadOutcome, AdminApiError> {
    let guard = state.config_write_lock();
    reload_locked(state, &guard)
}

/// [`reload_runtime`] 的无锁版本：调用方必须已持有 `config_write_lock` 守卫
/// （provider compile 的读改写全程持锁，不能嵌套取锁）。
pub fn reload_locked(
    state: &ControlState,
    _guard: &std::sync::MutexGuard<'_, ()>,
) -> Result<ReloadOutcome, AdminApiError> {
    let file = read_config_file(state)?;
    let restart_required = state
        .running_server()
        .is_some_and(|running| running != file.server);
    let catalog = into_catalog_for_state(state, &file).map_err(|error| {
        AdminApiError::with_issues(
            ApiErrorType::ConfigInvalid,
            error.to_string(),
            vec![issue_from_config_error(&error)],
        )
    })?;
    let report = state.pipeline.reload(catalog);
    Ok(ReloadOutcome {
        revision: report.digest,
        previous_revision: report.previous_digest,
        restart_required,
    })
}

/// 拒绝把 GET 文档里的脱敏占位符（`redacted:literal` 等）原样写回磁盘：
/// 一旦回写，真实凭据引用就被覆盖成了无意义的占位串。`env:`/`file:`/`keyring:`
/// 引用与既有 literal 均不受影响（GET 视图只改写 literal 的展示，apply 需要前端
/// 用 secrets 之外的原始值——即不动 credential 字段或传新引用）。
fn reject_redacted_placeholders(file: &ConfigFile) -> Result<(), AdminApiError> {
    let placeholder = "redacted:literal";
    let mut hits: Vec<String> = Vec::new();
    for candidate in &file.candidates {
        if candidate.credential.as_deref() == Some(placeholder) {
            hits.push(format!("candidates.{}.credential", candidate.id));
        }
    }
    if file.judge.service.credential.as_deref() == Some(placeholder) {
        hits.push("judge.service.credential".to_string());
    }
    for virtual_model in &file.virtual_models {
        if virtual_model.judge.service.credential.as_deref() == Some(placeholder) {
            hits.push(format!(
                "virtual_models.{}.judge.service.credential",
                virtual_model.id
            ));
        }
    }
    if file.server.admin_token == placeholder || file.server.inbound_key == placeholder {
        hits.push("server.admin_token/inbound_key".to_string());
    }
    if hits.is_empty() {
        return Ok(());
    }
    Err(AdminApiError::with_issues(
        ApiErrorType::ConfigInvalid,
        format!(
            "redacted placeholder(s) {hits:?} cannot be written back; \
             keep the original value, use a env:/file: reference, or set a new credential"
        ),
        vec![ConfigIssue {
            path: hits.join(","),
            code: "redacted_placeholder".to_string(),
            message: "提交内容包含 redacted:literal 占位符；请保留原值或改用 env:/file: 引用"
                .to_string(),
        }],
    ))
}

/// 窗口内最近的一份备份（复用，避免自动保存烧穿回滚环形）。
fn recent_backup(path: &std::path::Path, now_micros: u128) -> Option<String> {
    let parent = path.parent()?;
    let base_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let prefix = format!("{base_name}.bak-");
    let mut best: Option<(u128, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Ok(stamp) = suffix.parse::<u128>() else {
            continue;
        };
        if now_micros.saturating_sub(stamp) > BACKUP_MIN_INTERVAL_MICROS {
            continue;
        }
        match &best {
            Some((current, _)) if *current >= stamp => {}
            _ => best = Some((stamp, entry.path())),
        }
    }
    best.map(|(_, path)| path.to_string_lossy().to_string())
}

/// 备份当前配置；返回备份路径。最多保留 MAX_BACKUPS 份。
///
/// 文件名 `.bak-<unix微秒>`；同一瞬间多次写盘会撞名，用微秒递增偏移 +
/// `create_new` 抢占唯一名（仍是纯数字，保持 rollback 按 stamp 排序取最新的语义）。
///
/// 自动保存下 apply 频率很高：距最近一份备份不足 [`BACKUP_MIN_INTERVAL_MICROS`]
/// 时复用那份，不新建——它代表本次编辑会话开始前的状态，节流不损失回滚语义。
fn backup_config(path: &std::path::Path) -> Result<Option<String>, AdminApiError> {
    if !path.is_file() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_micros())
        .unwrap_or(0);
    if let Some(recent) = recent_backup(path, stamp) {
        return Ok(Some(recent));
    }
    let base_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut backup = path.with_file_name(format!("{base_name}.bak-{stamp}"));
    // create_new 抢占：撞名/精度碰撞时递增重试（有界）。
    let mut content = None;
    for offset in 0..1000u128 {
        backup = path.with_file_name(format!("{base_name}.bak-{}", stamp + offset));
        let open = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup);
        match open {
            Ok(file) => {
                content = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(AdminApiError::internal(format!("backup config: {error}")));
            }
        }
    }
    let mut file = content.ok_or_else(|| {
        AdminApiError::internal("backup config: cannot acquire unique backup name")
    })?;
    std::io::copy(
        &mut std::fs::File::open(path)
            .map_err(|error| AdminApiError::internal(format!("backup config: {error}")))?,
        &mut file,
    )
    .map_err(|error| AdminApiError::internal(format!("backup config: {error}")))?;
    prune_backups(path, MAX_BACKUPS)?;
    Ok(Some(backup.to_string_lossy().to_string()))
}

fn prune_backups(path: &std::path::Path, keep: usize) -> Result<(), AdminApiError> {
    let parent = path
        .parent()
        .ok_or_else(|| AdminApiError::internal("config path has no parent"))?;
    let base_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut backups: Vec<(u64, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(parent)
        .map_err(|error| AdminApiError::internal(error.to_string()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(stamp) = name
            .strip_prefix(&format!("{base_name}.bak-"))
            .and_then(|suffix| suffix.parse::<u64>().ok())
        {
            backups.push((stamp, entry.path()));
        }
    }
    backups.sort_by_key(|(stamp, _)| *stamp);
    while backups.len() > keep {
        let (_, oldest) = backups.remove(0);
        let _ = std::fs::remove_file(oldest);
    }
    Ok(())
}

/// 递归脱敏文档：凭据字段只留引用；返回发现的 secrets。
fn redact_document(value: &mut serde_json::Value) -> Vec<SecretRef> {
    let mut secrets = Vec::new();
    redact_value(value, &mut Vec::new(), &mut secrets);
    secrets
}

fn redact_value(
    value: &mut serde_json::Value,
    path: &mut Vec<String>,
    secrets: &mut Vec<SecretRef>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "credential" || key == "admin_token" || key == "inbound_key" {
                    if let Some(reference) = redact_credential(child) {
                        let path_string = format!("{}{}", path.join("."), key);
                        secrets.push(SecretRef {
                            path: path_string,
                            reference,
                        });
                    }
                } else {
                    path.push(key.clone());
                    redact_value(child, path, secrets);
                    path.pop();
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter_mut().enumerate() {
                path.push(format!("[{index}]"));
                redact_value(child, path, secrets);
                path.pop();
            }
        }
        _ => {}
    }
}

/// 单个凭据字段的脱敏：返回引用描述。
/// 引用类型（`env:`/`file:`/`keyring:`）原样保留；字面量替换占位符。
fn redact_credential(value: &mut serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => {
            if is_credential_reference(text) {
                Some(text.clone())
            } else if text.is_empty() {
                None
            } else {
                *value = serde_json::Value::String("redacted:literal".to_string());
                Some("redacted:literal".to_string())
            }
        }
        _ => None,
    }
}

/// 是否是凭据引用（不是明文字面量）：`env:` / `file:` / `keyring:`。
/// keyring 是内核 CredentialRef 合法类型（`CredentialRef::keyring`），必须与
/// env/file 同等对待：脱敏视图原样保留、apply/rollback 不当占位符拒绝。
fn is_credential_reference(text: &str) -> bool {
    text.starts_with("env:") || text.starts_with("file:") || text.starts_with("keyring:")
}

/// 序列化 TOML 前做与 GET 文档一致的脱敏：结构化 clone ConfigFile，
/// 显式替换敏感字段（不依赖 TOML 文本替换——序列化器对含引号/换行的
/// 字符串会用单引号或三引号字面量，逐行识别必然漏）。`env:`/`file:`/
/// `keyring:` 引用原样保留。
fn redact_toml(file: &ConfigFile) -> String {
    let mut redacted = file.clone();
    // server 凭据。
    if is_secret_value(&redacted.server.admin_token) {
        redacted.server.admin_token = PLACEHOLDER.into();
    }
    if is_secret_value(&redacted.server.inbound_key) {
        redacted.server.inbound_key = PLACEHOLDER.into();
    }
    // judge.service.credential。
    if let Some(credential) = redacted.judge.service.credential.as_mut() {
        if is_secret_value(credential) {
            *credential = PLACEHOLDER.into();
        }
    }
    // [[candidates]].credential。
    for candidate in &mut redacted.candidates {
        if let Some(credential) = candidate.credential.as_mut() {
            if is_secret_value(credential) {
                *credential = PLACEHOLDER.into();
            }
        }
    }
    // [[virtual_models]].judge.service.credential。
    for virtual_model in &mut redacted.virtual_models {
        if let Some(credential) = virtual_model.judge.service.credential.as_mut() {
            if is_secret_value(credential) {
                *credential = PLACEHOLDER.into();
            }
        }
    }
    toml::to_string(&redacted).unwrap_or_default()
}

/// 脱敏占位符（与 GET JSON 视图的 `redact_document` 同值）。
const PLACEHOLDER: &str = "redacted:literal";

/// 是否是需要脱敏的字面量（引用与占位符/空值保留原样）。
fn is_secret_value(value: &str) -> bool {
    !(is_credential_reference(value) || value == PLACEHOLDER || value.is_empty())
}

/// 简单 diff：顶层字段级比较。旧值取**写之前的磁盘文档**（审计/展示一致）。
fn diff_documents_disk(old_file: &ConfigFile, new_file: &ConfigFile) -> Vec<ConfigDiffEntry> {
    let old_value = serde_json::json!({
        "default_virtual_model": old_file.default_virtual_model,
        "judge": old_file.judge.mode,
        "cost_gate": old_file.cost_gate.mode,
        "candidates_count": old_file.candidates.len(),
        "virtual_models_count": old_file.virtual_models.len(),
    });
    let new_value = serde_json::json!({
        "default_virtual_model": new_file.default_virtual_model,
        "judge": new_file.judge.mode,
        "cost_gate": new_file.cost_gate.mode,
        "candidates_count": new_file.candidates.len(),
        "virtual_models_count": new_file.virtual_models.len(),
    });
    let mut diff = Vec::new();
    if let (serde_json::Value::Object(old_map), serde_json::Value::Object(new_map)) =
        (&old_value, &new_value)
    {
        for (key, old_field) in old_map {
            if new_map.get(key) != Some(old_field) {
                diff.push(ConfigDiffEntry {
                    path: key.clone(),
                    from: Some(old_field.clone()),
                    to: new_map.get(key).cloned(),
                });
            }
        }
    }
    diff
}

fn issue_from_config_error(error: &tr_agent::config::ConfigError) -> ConfigIssue {
    let (code, message) = match error {
        tr_agent::config::ConfigError::SchemaVersion { found, supported } => (
            "schema_version",
            format!("unsupported schema_version {found}; supported {supported}"),
        ),
        tr_agent::config::ConfigError::Invalid(text) => ("invalid_value", text.clone()),
        tr_agent::config::ConfigError::Toml(toml_error) => ("parse_error", toml_error.to_string()),
        tr_agent::config::ConfigError::Io(io_error) => ("io_error", io_error.to_string()),
    };
    ConfigIssue {
        path: String::new(),
        code: code.to_string(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
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

    fn file_with(credential: &str) -> ConfigFile {
        let toml = MINIMAL.replacen(
            "input = [\"text\"]",
            &format!(
                "input = [\"text\"]\ncredential = \"{}\"",
                toml_escape(credential)
            ),
            1,
        );
        ConfigFile::parse(&toml).expect("parse")
    }

    /// TOML basic string 转义（测试构造用；覆盖引号/反斜杠/控制字符）。
    fn toml_escape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                other => out.push(other),
            }
        }
        out
    }

    #[test]
    fn redact_toml_masks_literals_keeps_refs() {
        let out = redact_toml(&file_with("sk-live-secret"));
        assert!(out.contains("credential = \"redacted:literal\""), "{out}");
        assert!(!out.contains("sk-live-secret"));

        let out = redact_toml(&file_with("env:OPENAI_KEY"));
        assert!(out.contains("credential = \"env:OPENAI_KEY\""), "{out}");

        let out = redact_toml(&file_with("file:credentials/a.key"));
        assert!(
            out.contains("credential = \"file:credentials/a.key\""),
            "{out}"
        );
    }

    /// 保密回归：含双引号/单引号/换行的明文必须被脱敏，不管 TOML 序列化器
    /// 选用哪种字面量形式（单引号 literal string / 三引号 multiline）。
    /// 旧的"逐行文本替换"实现会漏这些形式。
    #[test]
    fn redact_toml_handles_quoting_and_newlines() {
        // 反斜杠转义的双引号（TOML basic string）。
        let secrets = [
            r#"has "quotes" inside"#,
            "multi\nline\nsecret",
            r#"both '"mixed'" tokens"#,
        ];
        for secret in &secrets {
            let out = redact_toml(&file_with(secret));
            assert!(
                out.contains("credential = \"redacted:literal\""),
                "secret {secret:?} leaked in normalized toml:\n{out}"
            );
            assert!(
                !out.contains(secret),
                "secret {secret:?} verbatim in normalized toml:\n{out}"
            );
        }
    }

    /// 保密回归：admin_token / inbound_key / judge.service.credential 同样脱敏。
    #[test]
    fn redact_toml_covers_all_secret_fields() {
        let mut file = file_with("env:REF_KEEP");
        file.server.admin_token = "tok-secret-123".into();
        file.server.inbound_key = "inb-secret-456".into();
        file.judge.service.credential = Some("judge-secret-789".into());

        let out = redact_toml(&file);
        assert!(out.contains("admin_token = \"redacted:literal\""), "{out}");
        assert!(out.contains("inbound_key = \"redacted:literal\""), "{out}");
        assert!(out.contains("credential = \"redacted:literal\""), "{out}");
        assert!(!out.contains("tok-secret-123"), "{out}");
        assert!(!out.contains("inb-secret-456"), "{out}");
        assert!(!out.contains("judge-secret-789"), "{out}");
        // 引用保留。
        assert!(
            out.contains("credential = \"env:REF_KEEP\"")
                || out.matches("env:REF_KEEP").count() >= 1,
            "{out}"
        );
    }

    /// 保密回归：group judge 的 credential 同样脱敏/拒绝占位符。
    #[test]
    fn group_judge_credential_is_redacted_and_placeholder_rejected() {
        let mut file = ConfigFile::parse(&format!(
            "{MINIMAL}\n\
             [virtual_models.judge]\n\
             mode = \"active\"\n\
             \n\
             [virtual_models.judge.service]\n\
             kind = \"llm\"\n\
             endpoint = \"http://127.0.0.1:11434/v1\"\n\
             model = \"judge\"\n\
             credential = \"group-secret-123\"\n"
        ))
        .expect("parse");
        let out = redact_toml(&file);
        assert!(!out.contains("group-secret-123"), "{out}");
        assert!(out.contains("credential = \"redacted:literal\""), "{out}");

        file.virtual_models[0].judge.service.credential = Some("redacted:literal".into());
        let error = reject_redacted_placeholders(&file).expect_err("must reject");
        assert!(
            error
                .message
                .contains("virtual_models.auto.judge.service.credential"),
            "{}",
            error.message
        );
    }

    #[test]
    fn redacted_placeholder_is_rejected_on_apply() {
        let file = file_with("redacted:literal");
        let error = reject_redacted_placeholders(&file).expect_err("must reject");
        assert_eq!(error.kind, ApiErrorType::ConfigInvalid);
        assert!(
            error.message.contains("redacted placeholder"),
            "{}",
            error.message
        );
        assert!(error.message.contains("candidates.a.credential"));

        // 引用（含 keyring）与空值不拒绝。
        let file = file_with("env:KEY");
        assert!(reject_redacted_placeholders(&file).is_ok());
        let file = file_with("keyring:openai-main");
        assert!(reject_redacted_placeholders(&file).is_ok());
        let file = file_with("");
        assert!(reject_redacted_placeholders(&file).is_ok());
    }

    #[test]
    fn diff_disk_base_compares_toml_files() {
        let old = file_with("env:OLD");
        let new = file_with("env:NEW");
        let diff = diff_documents_disk(&old, &new);
        assert!(diff.is_empty(), "credential 变化不进顶层 diff");

        let mut new_file = new.clone();
        new_file.judge.mode = "active".to_string();
        let diff = diff_documents_disk(&old, &new_file);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].path, "judge");
    }

    #[test]
    fn relative_file_credentials_resolve_against_config_base_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("credentials")).expect("create credentials dir");
        std::fs::write(dir.path().join("credentials/judge.key"), "sk-relative").expect("write key");

        let toml = format!(
            "{MINIMAL}\n\
             [virtual_models.judge]\n\
             mode = \"active\"\n\
             \n\
             [virtual_models.judge.service]\n\
             kind = \"llm\"\n\
             endpoint = \"http://127.0.0.1:11434/v1\"\n\
             model = \"judge\"\n\
             credential = \"file:credentials/judge.key\"\n"
        );
        let file = ConfigFile::parse(&toml).expect("parse");

        // 有基准目录：相对 file: 引用可解析（apply/validate 走这条路径）。
        into_catalog_with_base(&file, Some(dir.path())).expect("resolve relative credential");
        // 没有基准目录：保持原来的失败语义，不静默吞掉缺失文件。
        assert!(into_catalog_with_base(&file, None).is_err());
    }
}
