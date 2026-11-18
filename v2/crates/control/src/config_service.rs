//! 配置服务（P4-031/032/033）：文档脱敏视图、validate、apply（CAS + 原子写 + 备份）、rollback。
//!
//! 硬规则（技术方案 §4.5/§4.7）：
//! - 不返回密钥值：`credential` 只返回 `env:`/`file:` 引用；字面量一律 `redacted:literal`；
//! - `revision` = 规范化 JSON 的 SHA-256（与账本 `config_digest` 同源）；
//! - `apply` 必须 `If-Match`；不匹配 → `409 config_conflict`；
//! - 原子写：临时文件 → rename；旧文件保留 `config.toml.bak-<unix>`（最多 10 份）；
//! - 服务端不自我重启：apply 只写文件并返回 `restart_required: true`。

use tr_agent::config::{Catalog, ConfigFile};
use tr_contracts::{ConfigDiffEntry, ConfigDocument, ConfigIssue, SecretRef, ValidateResponse};

use crate::error::{AdminApiError, ApiErrorType};
use crate::state::ControlState;

/// 备份保留上限。
const MAX_BACKUPS: usize = 10;

/// `GET /v2/admin/config`：脱敏文档视图。
pub fn config_document(
    catalog: &Catalog,
    state: &ControlState,
) -> Result<ConfigDocument, AdminApiError> {
    // 从配置文件重新读原文（pipeline 里的是运行时 Catalog，已经过归一）。
    let path = state
        .config_path
        .as_ref()
        .ok_or_else(|| AdminApiError::internal("config path not set"))?;
    let text = std::fs::read_to_string(path)
        .map_err(|error| AdminApiError::internal(format!("read config: {error}")))?;
    let file = ConfigFile::parse(&text)
        .map_err(|error| AdminApiError::internal(format!("parse config: {error}")))?;
    let mut document_value =
        serde_json::to_value(&file).map_err(|error| AdminApiError::internal(error.to_string()))?;
    let secrets = redact_document(&mut document_value);
    Ok(ConfigDocument {
        schema_version: file.schema_version,
        revision: catalog.config_digest.clone(),
        editable: true,
        document: document_value,
        secrets,
        warnings: Vec::new(),
    })
}

/// `POST /v2/admin/config/validate`：纯校验（不写盘）。
pub fn validate(
    catalog: &Catalog,
    _state: &ControlState,
    document_toml: &str,
) -> Result<ValidateResponse, AdminApiError> {
    match ConfigFile::parse(document_toml) {
        Ok(file) => match file.clone().into_catalog() {
            Ok(new_catalog) => {
                let mut document_value = serde_json::to_value(&file)
                    .map_err(|error| AdminApiError::internal(error.to_string()))?;
                let _ = redact_document(&mut document_value);
                let diff = diff_documents(catalog, &file)?;
                Ok(ValidateResponse {
                    valid: true,
                    revision: Some(new_catalog.config_digest),
                    normalized_toml: Some(normalize_toml(&file)),
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
pub struct ApplyOutcome {
    pub revision: String,
    pub previous_revision: Option<String>,
    pub backup_path: Option<String>,
    pub diff: Vec<ConfigDiffEntry>,
}

/// `POST /v2/admin/config/apply`：CAS + 原子写 + 备份。
pub fn apply(
    catalog: &Catalog,
    state: &ControlState,
    document_toml: &str,
    expected_revision: Option<&str>,
) -> Result<ApplyOutcome, AdminApiError> {
    // CAS：If-Match 必须匹配当前 revision。
    if let Some(expected) = expected_revision {
        if expected != catalog.config_digest {
            return Err(AdminApiError::new(
                ApiErrorType::ConfigConflict,
                format!(
                    "revision mismatch: expected {expected}, current {}",
                    catalog.config_digest
                ),
            ));
        }
    }

    let path = state
        .config_path
        .as_ref()
        .ok_or_else(|| AdminApiError::internal("config path not set"))?;

    // 先解析 + 校验（apply 返回 422；validate 返回 200+valid:false）。
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
    let new_catalog = file.clone().into_catalog().map_err(|error| {
        AdminApiError::with_issues(
            ApiErrorType::ConfigInvalid,
            error.to_string(),
            vec![issue_from_config_error(&error)],
        )
    })?;

    let diff = diff_documents(catalog, &file)?;
    let previous_revision = catalog.config_digest.clone();

    // 写入前备份。
    let backup_path = backup_config(path)?;

    // 原子写：同目录临时文件 → rename。
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let temp = path.with_extension(format!("toml.tmp-{stamp}"));
    std::fs::write(&temp, document_toml.as_bytes())
        .map_err(|error| AdminApiError::internal(format!("write temp: {error}")))?;
    std::fs::rename(&temp, path)
        .map_err(|error| AdminApiError::internal(format!("rename config: {error}")))?;

    // 写后读回校验（防半写）。
    if let Err(error) = ConfigFile::parse(
        &std::fs::read_to_string(path)
            .map_err(|error| AdminApiError::internal(error.to_string()))?,
    ) {
        // 读回失败：回滚。
        let _ = std::fs::copy(backup_path.as_deref().unwrap_or_default(), path);
        return Err(AdminApiError::internal(format!(
            "post-write verification failed, rolled back: {error}"
        )));
    }

    Ok(ApplyOutcome {
        revision: new_catalog.config_digest,
        previous_revision: Some(previous_revision),
        backup_path,
        diff,
    })
}

/// `POST /v2/admin/config/rollback`：还原最近一份备份。
pub fn rollback(state: &ControlState) -> Result<ApplyOutcome, AdminApiError> {
    let path = state
        .config_path
        .as_ref()
        .ok_or_else(|| AdminApiError::internal("config path not set"))?;
    let parent = path
        .parent()
        .ok_or_else(|| AdminApiError::internal("config path has no parent"))?;
    let base_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.toml".to_string());

    // 找最新的 .bak-<unix>。
    let mut backups: Vec<(u64, std::path::PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
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
    let Some((_, latest)) = backups.pop() else {
        return Err(AdminApiError::not_found("no backup to roll back to"));
    };

    let text = std::fs::read_to_string(&latest)
        .map_err(|error| AdminApiError::internal(format!("read backup: {error}")))?;
    let file = ConfigFile::parse(&text)
        .map_err(|error| AdminApiError::internal(format!("backup is invalid: {error}")))?;
    let new_catalog = file
        .into_catalog()
        .map_err(|error| AdminApiError::internal(format!("backup is invalid: {error}")))?;

    std::fs::write(path, &text)
        .map_err(|error| AdminApiError::internal(format!("restore backup: {error}")))?;
    Ok(ApplyOutcome {
        revision: new_catalog.config_digest,
        previous_revision: None,
        backup_path: Some(latest.to_string_lossy().to_string()),
        diff: Vec::new(),
    })
}

/// 备份当前配置；返回备份路径。最多保留 MAX_BACKUPS 份。
fn backup_config(path: &std::path::Path) -> Result<Option<String>, AdminApiError> {
    if !path.is_file() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let backup = path.with_file_name(format!(
        "{}.bak-{stamp}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::copy(path, &backup)
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
fn redact_credential(value: &mut serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => {
            if text.starts_with("env:") || text.starts_with("file:") {
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

/// 规范化 TOML：serde 序列化（键序稳定）。
fn normalize_toml(file: &ConfigFile) -> String {
    toml::to_string(file).unwrap_or_default()
}

/// 简单 diff：顶层字段级比较（P4-032 只要求字段路径可映射；深度 diff 在 UI 层展示）。
fn diff_documents(
    old: &Catalog,
    new_file: &ConfigFile,
) -> Result<Vec<ConfigDiffEntry>, AdminApiError> {
    let old_value = serde_json::json!({
        "default_virtual_model": old.default_virtual_model,
        "judge": mode_label(&old.judge.mode),
        "cost_gate": mode_label_cost(&old.cost_gate),
        "candidates_count": old.candidates.len(),
        "virtual_models_count": old.virtual_models.len(),
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
    Ok(diff)
}

fn mode_label(mode: &tr_agent::judge::JudgeMode) -> &'static str {
    match mode {
        tr_agent::judge::JudgeMode::Off => "off",
        tr_agent::judge::JudgeMode::Shadow => "shadow",
        tr_agent::judge::JudgeMode::Active => "active",
    }
}

fn mode_label_cost(mode: &tr_agent::gate::CostGateMode) -> &'static str {
    match mode {
        tr_agent::gate::CostGateMode::Off => "off",
        tr_agent::gate::CostGateMode::Shadow => "shadow",
        tr_agent::gate::CostGateMode::Active => "active",
    }
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
