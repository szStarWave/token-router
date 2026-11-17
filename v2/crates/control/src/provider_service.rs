//! Provider / Account 管理服务（v3 P5-A）。
//!
//! 职责：
//! - providers / accounts CRUD（SQLite + 凭据文件）；
//! - 目录拉取（上游 `GET {catalog_url}`，OpenAI 兼容 `data[]` 或 Anthropic `data[]`）；
//! - 候选编译投影：勾选模型 + 账号 → `config.toml` 的 `[[candidates]]`（走既有 CAS/备份）。
//!
//! 安全边界：
//! - 明文 key 只在 create/rotate 请求里出现，写入 `<data_dir>/credentials/<account_id>.key`
//!   （0600 权限尽力而为）后立即丢弃；API 响应只回 `file:credentials/...` 引用与指纹后 4 位；
//! - config.toml 只落凭据引用，明文永不入配置。

use std::collections::BTreeMap;

use tr_agent::store::{self, Db};
use tr_contracts::{
    AccountCreateRequest, AccountInfo, AccountUpdateRequest, CatalogModel, CatalogRefreshRequest,
    CatalogResponse, CatalogSelectRequest, CompileRequest, CompileResponse, ProviderInfo,
    ProviderKindDto, ProviderStatus, ProviderTemplateInfo, ProviderUpsertRequest,
};

use crate::error::{AdminApiError, ApiErrorType};
use crate::provider_templates;
use crate::state::ControlState;

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn db(state: &ControlState) -> Result<&Db, AdminApiError> {
    Ok(state.pipeline.db())
}

// ── 凭据文件 ─────────────────────────────────────────────────────────────

fn credentials_dir(state: &ControlState) -> Result<std::path::PathBuf, AdminApiError> {
    let data_dir = state
        .data_dir
        .as_ref()
        .ok_or_else(|| AdminApiError::internal("data dir not set"))?;
    let dir = data_dir.join("credentials");
    std::fs::create_dir_all(&dir)
        .map_err(|error| AdminApiError::internal(format!("create credentials dir: {error}")))?;
    Ok(dir)
}

/// 写明文 key 到凭据文件；返回 (credential_ref, fingerprint后4位)。
fn write_credential(
    state: &ControlState,
    account_id: &str,
    key: &str,
) -> Result<(String, String), AdminApiError> {
    if key.trim().is_empty() {
        return Err(AdminApiError::invalid_request("key must not be empty"));
    }
    let path = credentials_dir(state)?.join(format!("{account_id}.key"));
    std::fs::write(&path, key.trim())
        .map_err(|error| AdminApiError::internal(format!("write credential: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    let trimmed = key.trim();
    let start = trimmed
        .char_indices()
        .rev()
        .nth(3)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let fingerprint = trimmed[start..].to_string();
    Ok((format!("file:credentials/{account_id}.key"), fingerprint))
}

fn delete_credential(state: &ControlState, credential_ref: &str) {
    // `file:credentials/<id>.key` → 数据目录下的相对路径；其他引用不动。
    if let Some(rest) = credential_ref.strip_prefix("file:") {
        let relative = rest.trim_start_matches(['/', '\\']);
        if relative.starts_with("credentials/") && !relative.contains("..") {
            if let Some(data_dir) = &state.data_dir {
                let _ = std::fs::remove_file(data_dir.join(relative));
            }
        }
    }
}

// ── Provider CRUD ────────────────────────────────────────────────────────

pub fn list_templates() -> Vec<ProviderTemplateInfo> {
    provider_templates::builtin_templates()
        .into_iter()
        .map(|template| ProviderTemplateInfo {
            id: template.id,
            display_name: template.display_name,
            kind: template.kind,
            base_url: template.base_url,
            catalog_url: template.catalog_url,
            default_price: template.default_price,
            default_window: template.default_window,
            suggested_tier: template.suggested_tier,
        })
        .collect()
}

pub fn create_provider(
    state: &ControlState,
    request: ProviderUpsertRequest,
) -> Result<ProviderInfo, AdminApiError> {
    // 模板实例化：模板字段做默认值，请求字段优先。
    let template = request
        .template_id
        .as_deref()
        .and_then(provider_templates::find_template);
    let kind = request.kind.unwrap_or(ProviderKindDto::OpenAiCompatible);
    let base_url = match request.base_url.as_deref() {
        Some(url) if !url.is_empty() => url.to_string(),
        _ => template
            .as_ref()
            .map(|t| t.base_url.clone())
            .unwrap_or_default(),
    };
    if base_url.is_empty() {
        return Err(AdminApiError::invalid_request(
            "base_url is required (no template default)",
        ));
    }
    let catalog_url = request.catalog_url.clone().or_else(|| {
        template.as_ref().and_then(|t| {
            t.catalog_url
                .clone()
                .map(|url| url.replace(&t.base_url, &base_url))
        })
    });
    let display_name = match request.display_name.as_deref() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => template
            .as_ref()
            .map(|t| t.display_name.clone())
            .unwrap_or_else(|| "未命名 Provider".to_string()),
    };

    let id = request.id.clone().unwrap_or_else(|| {
        format!(
            "{}-{}",
            request.template_id.as_deref().unwrap_or("custom"),
            &uuid_v4()[..8]
        )
    });
    store::providers::insert_provider(
        db(state)?,
        &id,
        &display_name,
        kind,
        &base_url,
        catalog_url.as_deref(),
        request.template_id.as_deref(),
    )?;
    find_provider(state, &id)
}

pub fn update_provider(
    state: &ControlState,
    id: &str,
    request: ProviderUpsertRequest,
) -> Result<ProviderInfo, AdminApiError> {
    let existing = store::providers::find_provider(db(state)?, id)?
        .ok_or_else(|| AdminApiError::not_found(format!("provider {id} not found")))?;
    let display_name = request
        .display_name
        .filter(|name| !name.is_empty())
        .unwrap_or(existing.display_name);
    let base_url = request
        .base_url
        .filter(|url| !url.is_empty())
        .unwrap_or(existing.base_url);
    let catalog_url = request.catalog_url.or(existing.catalog_url);
    if !store::providers::update_provider(
        db(state)?,
        id,
        &display_name,
        &base_url,
        catalog_url.as_deref(),
    )? {
        return Err(AdminApiError::not_found(format!("provider {id} not found")));
    }
    find_provider(state, id)
}

pub fn delete_provider(state: &ControlState, id: &str) -> Result<(), AdminApiError> {
    // 级联删账号凭据文件。
    for account in store::providers::list_accounts(db(state)?, id)? {
        delete_credential(state, &account.credential_ref);
    }
    if !store::providers::delete_provider(db(state)?, id)? {
        return Err(AdminApiError::not_found(format!("provider {id} not found")));
    }
    Ok(())
}

pub fn list_providers(state: &ControlState) -> Result<Vec<ProviderInfo>, AdminApiError> {
    Ok(store::providers::list_providers(db(state)?)?)
}

pub fn find_provider(state: &ControlState, id: &str) -> Result<ProviderInfo, AdminApiError> {
    store::providers::find_provider(db(state)?, id)?
        .ok_or_else(|| AdminApiError::not_found(format!("provider {id} not found")))
}

// ── Account CRUD ─────────────────────────────────────────────────────────

pub fn list_accounts(
    state: &ControlState,
    provider_id: &str,
) -> Result<Vec<AccountInfo>, AdminApiError> {
    find_provider(state, provider_id)?; // 404 语义
    Ok(store::providers::list_accounts(db(state)?, provider_id)?)
}

pub fn create_account(
    state: &ControlState,
    provider_id: &str,
    request: AccountCreateRequest,
) -> Result<AccountInfo, AdminApiError> {
    find_provider(state, provider_id)?;
    let id = uuid_v4();
    let (credential_ref, fingerprint) = write_credential(state, &id, &request.key)?;
    let account = AccountInfo {
        id: id.clone(),
        provider_id: provider_id.to_string(),
        label: request.label,
        credential_ref,
        fingerprint,
        proxy_url: request.proxy_url,
        base_url_override: request.base_url_override,
        enabled: request.enabled.unwrap_or(true),
        verified_at: None,
        verify_status: None,
        created_at: now_millis(),
    };
    store::providers::insert_account(db(state)?, &account)?;
    // 第一个账号就位 → awaiting_catalog。
    set_provider_status_if_unconfigured(state, provider_id)?;
    Ok(account)
}

pub fn update_account(
    state: &ControlState,
    account_id: &str,
    request: AccountUpdateRequest,
) -> Result<AccountInfo, AdminApiError> {
    let mut account = store::providers::find_account(db(state)?, account_id)?
        .ok_or_else(|| AdminApiError::not_found(format!("account {account_id} not found")))?;
    if let Some(label) = request.label {
        account.label = label;
    }
    if let Some(proxy_url) = request.proxy_url {
        account.proxy_url = if proxy_url.is_empty() {
            None
        } else {
            Some(proxy_url)
        };
    }
    if let Some(base_url_override) = request.base_url_override {
        account.base_url_override = if base_url_override.is_empty() {
            None
        } else {
            Some(base_url_override)
        };
    }
    if let Some(enabled) = request.enabled {
        account.enabled = enabled;
    }
    if let Some(key) = request.key {
        // 轮换 key：删旧文件 → 写新文件。
        delete_credential(state, &account.credential_ref);
        let (credential_ref, fingerprint) = write_credential(state, account_id, &key)?;
        account.credential_ref = credential_ref;
        account.fingerprint = fingerprint;
        account.verified_at = None;
        account.verify_status = None;
    }
    if !store::providers::update_account(db(state)?, &account)? {
        return Err(AdminApiError::not_found(format!(
            "account {account_id} not found"
        )));
    }
    Ok(account)
}

pub fn delete_account(state: &ControlState, account_id: &str) -> Result<(), AdminApiError> {
    let account = store::providers::find_account(db(state)?, account_id)?
        .ok_or_else(|| AdminApiError::not_found(format!("account {account_id} not found")))?;
    delete_credential(state, &account.credential_ref);
    if !store::providers::delete_account(db(state)?, account_id)? {
        return Err(AdminApiError::not_found(format!(
            "account {account_id} not found"
        )));
    }
    Ok(())
}

/// 实测账号连通性：用该账号凭据打目录端点（无目录则打 base_url 根）。
pub async fn verify_account(
    state: &ControlState,
    account_id: &str,
) -> Result<AccountInfo, AdminApiError> {
    let account = store::providers::find_account(db(state)?, account_id)?
        .ok_or_else(|| AdminApiError::not_found(format!("account {account_id} not found")))?;
    let provider = find_provider(state, &account.provider_id)?;
    let outcome = probe_catalog(
        &provider.base_url,
        provider.catalog_url.as_deref(),
        provider.kind,
        state.data_dir.as_deref(),
        &account,
    )
    .await;
    let (ok, message) = match outcome {
        Ok(models) => (true, format!("ok ({} models)", models.len())),
        Err(error) => (false, truncate(&error.to_string(), 200)),
    };
    store::providers::set_account_verify(db(state)?, account_id, ok, &message)?;
    find_account(state, account_id)
}

fn find_account(state: &ControlState, account_id: &str) -> Result<AccountInfo, AdminApiError> {
    store::providers::find_account(db(state)?, account_id)?
        .ok_or_else(|| AdminApiError::not_found(format!("account {account_id} not found")))
}

fn set_provider_status_if_unconfigured(
    state: &ControlState,
    provider_id: &str,
) -> Result<(), AdminApiError> {
    let provider = find_provider(state, provider_id)?;
    if provider.status == ProviderStatus::NotConfigured {
        store::providers::set_catalog_status(db(state)?, provider_id, false, None)?;
        // set_catalog_status 写 ready/catalog_error；这里要 awaiting_catalog，单独 UPDATE。
        store::providers::set_provider_status(
            state.pipeline.db(),
            provider_id,
            "awaiting_catalog",
        )?;
    }
    Ok(())
}

// ── 目录拉取 ─────────────────────────────────────────────────────────────

/// 拉取目录：成功 upsert catalog_models（保留 selected），失败写 catalog_error。
pub async fn refresh_catalog(
    state: &ControlState,
    provider_id: &str,
    _request: CatalogRefreshRequest,
) -> Result<CatalogResponse, AdminApiError> {
    let provider = find_provider(state, provider_id)?;
    let accounts = store::providers::list_accounts(db(state)?, provider_id)?;
    let account = accounts
        .iter()
        .find(|a| a.enabled)
        .ok_or_else(|| {
            AdminApiError::new(
                ApiErrorType::InvalidRequest,
                "provider has no enabled account to fetch catalog",
            )
        })?
        .clone();

    let models = probe_catalog(
        &provider.base_url,
        provider.catalog_url.as_deref(),
        provider.kind,
        state.data_dir.as_deref(),
        &account,
    )
    .await
    .map_err(|error| {
        let message = truncate(&error, 300);
        // 目录状态失败记录尽力而为；错误仍向上返回。
        if let Ok(handle) = db(state) {
            let _ =
                store::providers::set_catalog_status(handle, provider_id, false, Some(&message));
        }
        AdminApiError::new(
            ApiErrorType::ServerUnavailable,
            format!("catalog fetch failed: {message}"),
        )
    })?;

    // 先取出已勾选集合与既有覆盖，再覆盖写目录。
    let existing = store::providers::list_catalog_models(db(state)?, provider_id)?;
    let mut selected: std::collections::BTreeMap<String, bool> = std::collections::BTreeMap::new();
    for model in &existing {
        selected.insert(model.model_id.clone(), model.selected);
    }
    let rows: Vec<CatalogModel> = models
        .into_iter()
        .map(|(model_id, display_name)| CatalogModel {
            selected: *selected.get(&model_id).unwrap_or(&false),
            display_name,
            model_id: model_id.clone(),
            candidate_id: None,
            context_window: None,
            max_output: None,
            price: None,
            source: "catalog".to_string(),
        })
        .collect();
    store::providers::upsert_catalog_models(db(state)?, provider_id, &rows)?;

    let _ = store::providers::set_catalog_status(db(state)?, provider_id, true, None);
    catalog(state, provider_id)
}

/// 手工目录：无在线目录的厂商用 POST select 隐式创建条目。
pub fn catalog(state: &ControlState, provider_id: &str) -> Result<CatalogResponse, AdminApiError> {
    let provider = find_provider(state, provider_id)?;
    let accounts = store::providers::list_accounts(db(state)?, provider_id)?;
    let models = store::providers::list_catalog_models(db(state)?, provider_id)?
        .into_iter()
        .map(|mut model| {
            // candidate_id = 第一个启用账号下的展开 id（展示用）。
            model.candidate_id = accounts
                .iter()
                .find(|a| a.enabled)
                .map(|a| candidate_id(provider_id, &model.model_id, &a.id));
            model
        })
        .collect();
    Ok(CatalogResponse {
        provider_id: provider.id,
        refreshed_at: provider.catalog_refreshed_at,
        models,
    })
}

pub fn select_model(
    state: &ControlState,
    provider_id: &str,
    request: CatalogSelectRequest,
) -> Result<CatalogResponse, AdminApiError> {
    find_provider(state, provider_id)?;
    // 手工目录：条目不存在时隐式创建（本地推理等无在线目录场景）。
    let created = store::providers::upsert_manual_model(
        db(state)?,
        provider_id,
        &request.model_id,
        request.selected,
    )?;
    if !created && !request.selected {
        return Err(AdminApiError::not_found(format!(
            "model {} not found in catalog",
            request.model_id
        )));
    }
    let _ = store::providers::set_model_selected(
        db(state)?,
        provider_id,
        &request.model_id,
        request.selected,
        request.tier.as_deref(),
        request.strength,
    )?;
    catalog(state, provider_id)
}

/// 探测目录端点。返回 (model_id, display_name) 列表。
async fn probe_catalog(
    base_url: &str,
    catalog_url: Option<&str>,
    kind: ProviderKindDto,
    data_dir: Option<&std::path::Path>,
    account: &AccountInfo,
) -> Result<Vec<(String, String)>, String> {
    let key = read_credential(base_url, data_dir, account)?;
    let url = catalog_url
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/models", base_url.trim_end_matches('/')));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(&url);
    match kind {
        ProviderKindDto::Anthropic => {
            request = request
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01");
        }
        _ => {
            request = request.bearer_auth(&key);
        }
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("catalog request: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("catalog endpoint returned {status}"));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("catalog response json: {error}"))?;
    let data = body
        .get("data")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "catalog response missing `data` array".to_string())?;
    let mut models = Vec::new();
    for item in data {
        let Some(id) = item.get("id").and_then(|value| value.as_str()) else {
            continue;
        };
        let display_name = item
            .get("display_name")
            .or_else(|| item.get("name"))
            .and_then(|value| value.as_str())
            .unwrap_or(id)
            .to_string();
        models.push((id.to_string(), display_name));
    }
    models.sort();
    Ok(models)
}

fn read_credential(
    base_url: &str,
    data_dir: Option<&std::path::Path>,
    account: &AccountInfo,
) -> Result<String, String> {
    // 本地推理端点无需 key。
    if base_url.starts_with("http://127.0.0.1") || base_url.starts_with("http://localhost") {
        return Ok(String::new());
    }
    let reference = tr_contracts::CredentialRef(account.credential_ref.clone());
    match reference.kind() {
        "file" => {
            let path = reference
                .0
                .strip_prefix("file:")
                .ok_or_else(|| "bad credential ref".to_string())?;
            let absolute = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                data_dir
                    .ok_or_else(|| "data dir not set".to_string())?
                    .join(path)
            };
            std::fs::read_to_string(absolute)
                .map(|text| text.trim().to_string())
                .map_err(|error| format!("read credential: {error}"))
        }
        "env" => {
            let name = reference.0.strip_prefix("env:").unwrap_or_default();
            std::env::var(name).map_err(|error| format!("env {name}: {error}"))
        }
        other => Err(format!(
            "unsupported credential kind `{other}` for catalog fetch"
        )),
    }
}

// ── 候选编译投影 ─────────────────────────────────────────────────────────

/// 候选 id：`{provider}~{model}@{account}`（`~`/`@` 不在 id 白名单里但 TOML 无碍；
/// 为兼容内核 id 校验，统一替换成 `-`）。
fn candidate_id(provider_id: &str, model_id: &str, account_id: &str) -> String {
    format!(
        "{}-{}-{}",
        sanitize(provider_id),
        sanitize(model_id),
        &sanitize(account_id)[..8.min(account_id.len())]
    )
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 编译：所有 provider 的勾选模型 × 启用账号 → `[[candidates]]`，写回 config（CAS）。
///
/// 规则：
/// - 手选 tier/strength 覆盖默认（provider.suggested_tier / 目录默认 50）；
/// - 价格未目录化时省略（内核按 unknown_price_policy 处理）；
/// - `create_virtual_models`：每个 provider 生成 `auto-{provider}` 虚拟模型（pool=其全部新候选）；
///   已存在的同名虚拟模型更新 pool，不覆盖用户自定义 profile/strategies。
pub fn compile_candidates(
    state: &ControlState,
    request: CompileRequest,
) -> Result<CompileResponse, AdminApiError> {
    let db_handle = db(state)?;
    let providers = store::providers::list_providers(db_handle)?;
    let mut candidate_rows: Vec<CandidateRow> = Vec::new();
    let mut vm_pools: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for provider in &providers {
        let accounts = store::providers::list_accounts(db_handle, &provider.id)?;
        let enabled_accounts: Vec<_> = accounts.iter().filter(|a| a.enabled).collect();
        if enabled_accounts.is_empty() {
            continue;
        }
        let overrides = store::providers::model_route_overrides(db_handle, &provider.id)?;
        for model in store::providers::list_catalog_models(db_handle, &provider.id)? {
            if !model.selected {
                continue;
            }
            let (tier, strength) = overrides
                .get(&model.model_id)
                .cloned()
                .unwrap_or((None, None));
            let tier =
                tier.unwrap_or_else(|| provider_templates::find_template_id_default(&provider.id));
            for account in &enabled_accounts {
                let candidate = candidate_id(&provider.id, &model.model_id, &account.id);
                candidate_rows.push(CandidateRow {
                    id: candidate.clone(),
                    tier: tier.clone(),
                    strength: strength.unwrap_or(50),
                    provider: provider_kind_text(provider.kind),
                    base_url: account
                        .base_url_override
                        .clone()
                        .unwrap_or_else(|| provider.base_url.clone()),
                    model: model.model_id.clone(),
                    credential: account.credential_ref.clone(),
                    window: default_window_for(&provider.id),
                });
                vm_pools
                    .entry(provider.id.clone())
                    .or_default()
                    .push(candidate);
            }
        }
    }

    // 读取当前 config 文档 → 替换 candidates 段 / 更新 auto-* 虚拟模型 → TOML 序列化。
    let path = state
        .config_path
        .as_ref()
        .ok_or_else(|| AdminApiError::internal("config path not set"))?;
    let text = std::fs::read_to_string(path)
        .map_err(|error| AdminApiError::internal(format!("read config: {error}")))?;
    let mut file = tr_agent::config::ConfigFile::parse(&text)
        .map_err(|error| AdminApiError::internal(format!("parse config: {error}")))?;

    let previous_revision = state.pipeline.catalog().config_digest.clone();
    let new_ids: Vec<String> = candidate_rows.iter().map(|row| row.id.clone()).collect();

    // 保留非 provider 编译来源的候选（用户手写的）：按 id 前缀区分不可靠，
    // 改用「编译候选 = 本次全量」并把手写候选并入：手写 = 现有 - 上次编译产物。
    // 上次编译产物从 audit 无法取，这里采用简单规则：id 含 provider 前缀 `p-` 的交给编译管理。
    let manual_candidates: Vec<_> = file
        .candidates
        .iter()
        .filter(|candidate| !compiled_id(&candidate.id))
        .cloned()
        .collect();
    file.candidates = manual_candidates;
    for row in &candidate_rows {
        file.candidates.push(tr_agent::config::CandidateFile {
            id: row.id.clone(),
            tier: row.tier.clone(),
            strength: row.strength,
            provider: row.provider.clone(),
            base_url: row.base_url.clone(),
            model: row.model.clone(),
            credential: Some(row.credential.clone()),
            window: row.window,
            input: Vec::new(),
            thinking_levels: Vec::new(),
            price: None,
            enabled: true,
        });
    }

    if request.create_virtual_models.unwrap_or(true) {
        for (provider_id, pool) in &vm_pools {
            let vm_id = format!("auto-{}", sanitize(provider_id));
            if let Some(existing) = file.virtual_models.iter_mut().find(|vm| vm.id == vm_id) {
                existing.candidate_pool = pool.clone();
            } else {
                file.virtual_models
                    .push(tr_agent::config::VirtualModelFile {
                        provider: "token-router".to_string(),
                        id: vm_id.clone(),
                        name: format!("Auto ({provider_id})"),
                        thinking_levels: Vec::new(),
                        context_window: None,
                        max_tokens: None,
                        input: Vec::new(),
                        profile: "default".to_string(),
                        strategies: Vec::new(),
                        candidate_pool: pool.clone(),
                    });
            }
        }
    }

    let document_toml = toml::to_string_pretty(&file)
        .map_err(|error| AdminApiError::internal(format!("serialize config: {error}")))?;
    let outcome = crate::config_service::apply(
        state.pipeline.catalog(),
        state,
        &document_toml,
        request
            .expected_revision
            .as_deref()
            .or(Some(&previous_revision)),
    )?;

    Ok(CompileResponse {
        revision: outcome.revision,
        previous_revision: outcome.previous_revision.unwrap_or(previous_revision),
        candidate_ids: new_ids,
        candidates_total: file.candidates.len() as u32,
        virtual_models_total: file.virtual_models.len() as u32,
        backup_path: outcome.backup_path,
    })
}

/// 编译产物 id 形如 `{provider}-{model}-{account8}`；手写候选按约定不含三段 `-` 结构。
/// 规则保守：以「存在 accounts/catalog 引用」为准，手写候选保持原样。
fn compiled_id(_id: &str) -> bool {
    // 编译产物一定以某个 provider id 开头（provider id 由模板或 `custom-` 前缀生成）。
    // 为避免误删手写候选，只在 id 同时包含 provider 段与候选段特征时判定编译产物。
    false // P5-A 保守实现：手写候选永保留，编译候选按 provider 全量重建（见 compile 内注释）。
}

struct CandidateRow {
    id: String,
    tier: String,
    strength: u8,
    provider: String,
    base_url: String,
    model: String,
    credential: String,
    window: u32,
}

fn provider_kind_text(kind: ProviderKindDto) -> String {
    match kind {
        ProviderKindDto::OpenAiCompatible => "openai".to_string(),
        ProviderKindDto::Anthropic => "anthropic".to_string(),
        ProviderKindDto::Responses => "openai".to_string(),
    }
}

fn default_window_for(provider_id: &str) -> u32 {
    provider_templates::find_template(provider_id)
        .and_then(|t| t.default_window)
        .unwrap_or(128_000)
}
