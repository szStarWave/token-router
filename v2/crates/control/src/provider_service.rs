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

use std::collections::{BTreeMap, BTreeSet};

use tr_agent::store::{self, Db};
use tr_contracts::{
    AccountCreateRequest, AccountInfo, AccountUpdateRequest, CatalogModel, CatalogRefreshRequest,
    CatalogResponse, CatalogSelectRequest, CompileRequest, CompileResponse, ModelPurposeDto,
    ProviderInfo, ProviderKindDto, ProviderStatus, ProviderTemplateInfo, ProviderUpsertRequest,
};
use tr_onboard::import;

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

/// 供应商 / 账号 / 目录变更后立即重建候选（尽力而为）。
///
/// 组页「添加模型」的可选列表来自编译后的 [[candidates]]；不自动编译就会出现
/// "供应商页已勾选、组页看不到"的错位。编译失败只记录日志：选择已落库，
/// 下一次变更会重试，不应让一次目录编辑整体失败。
fn compile_after_provider_change(state: &ControlState) {
    if state.config_path.is_none() {
        return; // 无配置文件（单元测试等）：没有编译目标
    }
    if let Err(error) = compile_candidates(
        state,
        CompileRequest {
            expected_revision: None,
        },
    ) {
        eprintln!("provider change applied but candidate compile failed: {error:?}");
    }
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
    if request.enabled == Some(false) {
        store::providers::set_provider_enabled(db(state)?, &id, false)?;
    }
    compile_after_provider_change(state);
    find_provider(state, &id)
}

pub fn update_provider(
    state: &ControlState,
    id: &str,
    request: ProviderUpsertRequest,
) -> Result<ProviderInfo, AdminApiError> {
    let existing = store::providers::find_provider(db(state)?, id)?
        .ok_or_else(|| AdminApiError::not_found(format!("provider {id} not found")))?;
    let enabled = request.enabled;
    let kind = request.kind.unwrap_or(existing.kind);
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
        kind,
        &base_url,
        catalog_url.as_deref(),
    )? {
        return Err(AdminApiError::not_found(format!("provider {id} not found")));
    }
    if let Some(enabled) = enabled {
        store::providers::set_provider_enabled(db(state)?, id, enabled)?;
    }
    if kind != existing.kind {
        // 类型变了：重建候选，并把仍是旧默认协议的候选迁到新协议。
        compile_after_provider_kind_change(state, id, existing.kind, kind);
    } else {
        compile_after_provider_change(state);
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
    compile_after_provider_change(state);
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
    let (credential_ref, fingerprint) = match (&request.key, &request.credential_ref) {
        (Some(key), _) if !key.trim().is_empty() => write_credential(state, &id, key)?,
        (None, Some(reference)) if !reference.trim().is_empty() => {
            // 只引用不复制：env:/file: 由内核解析；指纹记引用类型。
            let kind = reference
                .split_once(':')
                .map(|(kind, _)| kind)
                .unwrap_or("ref");
            (reference.trim().to_string(), format!("({kind})"))
        }
        _ => {
            return Err(AdminApiError::invalid_request(
                "key or credential_ref required",
            ));
        }
    };
    let account = AccountInfo {
        id: id.clone(),
        provider_id: provider_id.to_string(),
        label: request.label,
        credential_ref,
        fingerprint,
        proxy_url: request.proxy_url,
        base_url_override: request.base_url_override,
        extra_headers: request.extra_headers,
        enabled: request.enabled.unwrap_or(true),
        verified_at: None,
        verify_status: None,
        created_at: now_millis(),
    };
    store::providers::insert_account(db(state)?, &account)?;
    // 第一个账号就位 → awaiting_catalog。
    set_provider_status_if_unconfigured(state, provider_id)?;
    compile_after_provider_change(state);
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
    if let Some(extra_headers) = request.extra_headers {
        account.extra_headers = if extra_headers.is_empty() {
            None
        } else {
            Some(extra_headers)
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
    compile_after_provider_change(state);
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
    compile_after_provider_change(state);
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

    // 非对话模型（embedding / rerank / 语音 / 图像生成）不出现在目录里：
    // 先用命名规则，再用 models.dev 的类型与输出模态兜底（缓存命中时几乎零成本）。
    let remote_non_chat = crate::modelsdev::non_chat_models(state, provider_id)
        .await
        .unwrap_or_default();
    let models: Vec<(String, String)> = models
        .into_iter()
        .filter(|(model_id, _)| !is_non_chat_model(model_id) && !remote_non_chat.contains(model_id))
        .collect();

    let fresh_ids: Vec<String> = models.iter().map(|(id, _)| id.clone()).collect();
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
            purpose: ModelPurposeDto::Chat,
            caps: Vec::new(),
            thinking_levels: Vec::new(),
            display_name,
            model_id: model_id.clone(),
            candidate_id: None,
            context_window: None,
            max_output: None,
            price: None,
            source: "catalog".to_string(),
            hidden: false,
        })
        .collect();
    store::providers::upsert_catalog_models(db(state)?, provider_id, &rows)?;

    // 历史刷新留下的 embedding / rerank 行：隐藏并取消勾选（数据保留，可恢复）。
    for model in &existing {
        if is_non_chat_model(&model.model_id) && !model.hidden {
            let _ =
                store::providers::set_model_hidden(db(state)?, provider_id, &model.model_id, true);
        }
    }

    // v3 P5-A 吸收项（roadmap §0.3）：auto-include 通配符自动勾选新模型；
    // 上游目录里消失的模型标记 deprecated（自动下架，不再编译出候选）。
    apply_auto_include(db(state)?, provider_id)?;
    mark_deprecated_models(db(state)?, provider_id, &fresh_ids);

    let _ = store::providers::set_catalog_status(db(state)?, provider_id, true, None);
    // models.dev 元数据补全（尽力而为）：窗口 / 输出 / 价格 / 推理档位只填空。
    crate::modelsdev::enrich_after_refresh(state, provider_id).await;
    // 已存在的非对话模型（历史刷新留下的语音/图像模型）：隐藏并取消勾选，可手动恢复。
    let _ = hide_non_chat_models(state, provider_id).await;
    // 目录刷新会改变「哪些模型是对话模型」（例如新过滤掉的 embedding / rerank、
    // 上游下架模型），立即重建候选，避免目录与路由组长时间不一致。
    compile_after_provider_change(state);
    catalog(state, provider_id)
}

/// 隐藏目录里的非对话模型（命名规则 + models.dev 类型/模态），返回隐藏数量。
async fn hide_non_chat_models(
    state: &ControlState,
    provider_id: &str,
) -> Result<u32, AdminApiError> {
    let remote_non_chat = crate::modelsdev::non_chat_models(state, provider_id).await?;
    let mut hidden = 0;
    for model in store::providers::list_catalog_models(db(state)?, provider_id)? {
        if model.hidden {
            continue;
        }
        if is_non_chat_model(&model.model_id) || remote_non_chat.contains(&model.model_id) {
            let _ = store::providers::set_model_selected(
                db(state)?,
                provider_id,
                &model.model_id,
                false,
                None,
                None,
            );
            let _ =
                store::providers::set_model_hidden(db(state)?, provider_id, &model.model_id, true);
            hidden += 1;
        }
    }
    Ok(hidden)
}

/// 按 provider 目录里配置的 auto-include 通配符（`gpt-*`）勾选新模型。
/// 通配符存在 `catalog_models.auto_include_pattern`（挂在 manual 条目上，
/// `POST catalog/select` 传 pattern 时写入）。
fn apply_auto_include(db: &tr_agent::store::Db, provider_id: &str) -> Result<(), AdminApiError> {
    let patterns = store::providers::auto_include_patterns(db, provider_id)?;
    if patterns.is_empty() {
        return Ok(());
    }
    for model in store::providers::list_catalog_models(db, provider_id)? {
        if model.selected || model.hidden {
            continue;
        }
        let hit = patterns
            .iter()
            .any(|pattern| wildcard_match(pattern, &model.model_id));
        if hit {
            let _ = store::providers::set_model_selected(
                db,
                provider_id,
                &model.model_id,
                true,
                None,
                None,
            );
        }
    }
    Ok(())
}

/// 目录刷新后：不在最新目录里的勾选模型标记 deprecated（下架不再编译）。
/// 模型仍留在目录里可手动恢复；模型重新出现时 deprecated 标记清除。
fn mark_deprecated_models(db: &tr_agent::store::Db, provider_id: &str, fresh_ids: &[String]) {
    let now = now_millis();
    let _ = store::providers::mark_deprecated(db, provider_id, fresh_ids, now);
}

/// magpie 口径的通配符匹配：只支持 `*`（任意段），避免引入正则依赖。
fn wildcard_match(pattern: &str, text: &str) -> bool {
    fn inner(p: &[u8], t: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        match p[0] {
            b'*' => {
                // 尝试 * 匹配 0..n 字符。
                for skip in 0..=t.len() {
                    if inner(&p[1..], &t[skip..]) {
                        return true;
                    }
                }
                false
            }
            byte => !t.is_empty() && t[0] == byte && inner(&p[1..], &t[1..]),
        }
    }
    inner(pattern.as_bytes(), text.as_bytes())
}

/// 设置 provider 级 auto-include 通配符（空串 = 清除），并立即匹配一轮。
pub fn set_auto_include(
    state: &ControlState,
    provider_id: &str,
    pattern: &str,
) -> Result<CatalogResponse, AdminApiError> {
    find_provider(state, provider_id)?;
    store::providers::set_provider_auto_include(db(state)?, provider_id, Some(pattern))
        .map_err(|error| AdminApiError::internal(error.to_string()))?;
    apply_auto_include(db(state)?, provider_id)?;
    compile_after_provider_change(state);
    let provider = find_provider(state, provider_id)?;
    let mut response = catalog(state, provider_id)?;
    // 附带当前 pattern 供 UI 回显。
    response.provider_id = provider.id;
    Ok(response)
}

/// 手工目录：无在线目录的厂商用 POST select 隐式创建条目。
pub fn catalog(state: &ControlState, provider_id: &str) -> Result<CatalogResponse, AdminApiError> {
    let provider = find_provider(state, provider_id)?;
    let accounts = store::providers::list_accounts(db(state)?, provider_id)?;
    let models = store::providers::list_catalog_models(db(state)?, provider_id)?
        .into_iter()
        .filter(|model| model.model_id != store::providers::AUTO_INCLUDE_SENTINEL)
        // embedding / rerank 不进入对话模型目录（含历史刷新留下的行）。
        .filter(|model| !is_non_chat_model(&model.model_id))
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
    // 手工目录：勾选一个不存在的条目时隐式创建（本地推理等无在线目录场景）。
    // 取消勾选只更新已有条目；不存在的条目返回 404。
    let exists =
        store::providers::catalog_model_exists(db(state)?, provider_id, &request.model_id)?;
    if !exists && !request.selected {
        return Err(AdminApiError::not_found(format!(
            "model {} not found in catalog",
            request.model_id
        )));
    }
    if !exists {
        store::providers::upsert_manual_model(
            db(state)?,
            provider_id,
            &request.model_id,
            request.selected,
        )?;
    }
    let _ = store::providers::set_model_selected(
        db(state)?,
        provider_id,
        &request.model_id,
        request.selected,
        request.tier.as_deref(),
        request.strength,
    )?;
    // 用途 / 能力标签 / 窗口 / 最大输出 / 价格是目录元数据（selected 行为不变）。
    if request.purpose.is_some()
        || request.caps.is_some()
        || request.context_window.is_some()
        || request.max_output.is_some()
        || request.price.is_some()
    {
        let patch = store::providers::ModelMetadataPatch {
            purpose: request.purpose,
            caps: request.caps.clone(),
            context_window: request.context_window,
            max_output: request.max_output,
            price: request.price.clone(),
        };
        let _ = store::providers::update_model_metadata(
            db(state)?,
            provider_id,
            &request.model_id,
            &patch,
        )?;
    }
    // auto-include 通配符挂在该条目上（空串 = 清除）。
    if let Some(pattern) = &request.auto_include_pattern {
        let pattern = pattern.trim();
        let _ = store::providers::set_auto_include_pattern(
            db(state)?,
            provider_id,
            &request.model_id,
            if pattern.is_empty() {
                None
            } else {
                Some(pattern)
            },
        );
        // 有通配符时立即匹配一轮。
        apply_auto_include(db(state)?, provider_id)?;
    }
    if request.clear_deprecated.unwrap_or(false) {
        let _ = store::providers::clear_deprecated(db(state)?, provider_id, &request.model_id);
    }
    if let Some(hidden) = request.hidden {
        store::providers::set_model_hidden(db(state)?, provider_id, &request.model_id, hidden)?;
    }
    compile_after_provider_change(state);
    catalog(state, provider_id)
}

/// 持久删除目录条目：隐藏且取消勾选；仍被配置中的组/规则引用时拒绝。
pub fn delete_catalog_model(
    state: &ControlState,
    provider_id: &str,
    model_id: &str,
) -> Result<CatalogResponse, AdminApiError> {
    find_provider(state, provider_id)?;
    if !store::providers::catalog_model_exists(db(state)?, provider_id, model_id)? {
        return Err(AdminApiError::not_found(format!(
            "model {model_id} not found in catalog"
        )));
    }
    let references = catalog_model_references(state, provider_id, model_id)?;
    if !references.is_empty() {
        return Err(AdminApiError::with_issues(
            ApiErrorType::ConfigConflict,
            format!(
                "模型 {model_id} 仍被以下位置引用，不能删除：{}",
                references.join("；")
            ),
            references
                .iter()
                .map(|reference| tr_contracts::ConfigIssue {
                    path: reference.clone(),
                    code: "catalog_model_referenced".to_string(),
                    message: "请先在虚拟模型中移除引用".to_string(),
                })
                .collect(),
        ));
    }
    store::providers::set_model_hidden(db(state)?, provider_id, model_id, true)?;
    compile_after_provider_change(state);
    catalog(state, provider_id)
}

/// 目录模型的候选 id 在配置中的引用（组候选池 / 规则 prefer）。
fn catalog_model_references(
    state: &ControlState,
    provider_id: &str,
    model_id: &str,
) -> Result<Vec<String>, AdminApiError> {
    let Some(path) = state.config_path.as_ref() else {
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(path)
        .map_err(|error| AdminApiError::internal(format!("read config: {error}")))?;
    let file = tr_agent::config::ConfigFile::parse(&text)
        .map_err(|error| AdminApiError::internal(format!("parse config: {error}")))?;
    let candidate_ids: std::collections::BTreeSet<String> =
        store::providers::list_accounts(db(state)?, provider_id)?
            .into_iter()
            .map(|account| candidate_id(provider_id, model_id, &account.id))
            .collect();
    let mut references = Vec::new();
    for virtual_model in &file.virtual_models {
        if virtual_model
            .candidate_pool
            .iter()
            .any(|candidate| candidate_ids.contains(candidate))
        {
            references.push(format!(
                "virtual_models.{}.candidate_pool",
                virtual_model.id
            ));
        }
        if virtual_model
            .rules
            .iter()
            .any(|rule| candidate_ids.contains(&rule.prefer))
        {
            references.push(format!("virtual_models.{}.rules", virtual_model.id));
        }
    }
    Ok(references)
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

pub(crate) fn read_credential(
    base_url: &str,
    data_dir: Option<&std::path::Path>,
    account: &AccountInfo,
) -> Result<String, String> {
    // 本地推理端点无需 key。
    if base_url.starts_with("http://127.0.0.1") || base_url.starts_with("http://localhost") {
        return Ok(String::new());
    }
    read_credential_ref(&account.credential_ref, data_dir)
}

/// 严格读取凭据：不看 base_url 是否本地（usage 接口即使是本机代理也需要 key）。
pub(crate) fn read_credential_required(
    data_dir: Option<&std::path::Path>,
    account: &AccountInfo,
) -> Result<String, String> {
    read_credential_ref(&account.credential_ref, data_dir)
}

fn read_credential_ref(
    reference_ref: &str,
    data_dir: Option<&std::path::Path>,
) -> Result<String, String> {
    let reference = tr_contracts::CredentialRef(reference_ref.to_string());
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

// ── 反向导入（v3 P5-A）────────────────────────────────────────────────────

/// 扫描本机 agent 配置生成 provider 草稿。
/// 明文 key 永不导入；`env:`/`file:` 引用原样保留。
pub fn import_scan(
    _state: &ControlState,
    request: tr_contracts::ImportScanRequest,
) -> Result<tr_contracts::ImportScanResponse, AdminApiError> {
    let home = match request.home {
        Some(home) if !home.is_empty() => std::path::PathBuf::from(home),
        // 默认宿主 home：走 onboard 的 Host 语义。
        _ => tr_onboard::Target::Host
            .home()
            .map_err(|error| AdminApiError::internal(error.to_string()))?,
    };
    let drafts = import::scan_home(&home)
        .into_iter()
        .map(|provider| tr_contracts::ImportedProviderDraft {
            display_name: provider.display_name,
            base_url: provider.base_url,
            credential_ref: provider.credential_ref,
            source: provider.source,
        })
        .collect();
    Ok(tr_contracts::ImportScanResponse { drafts })
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
/// - **只写候选，不建路由组**：虚拟模型（路由组）一律由用户手动创建与维护，
///   编译不会新增、改写或删除 `[[virtual_models]]`。
///
/// 可靠性：读盘 → 改 → 写全程持有 [`ControlState::config_write_lock`]，
/// 与 config apply/rollback 串行；CAS 基准是当前磁盘 revision（If-Match 语义，
/// 请求缺省时用磁盘当前值兜底，仍会被校验）。
pub fn compile_candidates(
    state: &ControlState,
    request: CompileRequest,
) -> Result<CompileResponse, AdminApiError> {
    let guard = state.config_write_lock();
    compile_candidates_locked(state, request, &guard)
}

/// [`compile_candidates`] 的无锁版本：调用方必须已持有 `config_write_lock` 守卫。
pub fn compile_candidates_locked(
    state: &ControlState,
    request: CompileRequest,
    guard: &std::sync::MutexGuard<'_, ()>,
) -> Result<CompileResponse, AdminApiError> {
    compile_candidates_inner(state, request, guard, None)
}

/// provider 类型变更后的编译：把该 provider 下仍是「旧类型默认协议」的候选迁到新协议。
///
/// 只迁移等于旧类型默认值的候选；用户显式改成别的协议（例如在 OpenAI 供应商下
/// 单独把某个模型设为 `responses`）保持不变。手工候选也按 id 前缀识别。
fn compile_after_provider_kind_change(
    state: &ControlState,
    provider_id: &str,
    old_kind: ProviderKindDto,
    new_kind: ProviderKindDto,
) {
    if state.config_path.is_none() {
        return; // 无配置文件（单元测试等）：没有编译目标
    }
    let guard = state.config_write_lock();
    let result = compile_candidates_inner(
        state,
        CompileRequest {
            expected_revision: None,
        },
        &guard,
        Some((provider_id, old_kind, new_kind)),
    );
    drop(guard);
    if let Err(error) = result {
        eprintln!("provider kind changed but candidate compile failed: {error:?}");
    }
}

fn compile_candidates_inner(
    state: &ControlState,
    request: CompileRequest,
    _guard: &std::sync::MutexGuard<'_, ()>,
    protocol_migration: Option<(&str, ProviderKindDto, ProviderKindDto)>,
) -> Result<CompileResponse, AdminApiError> {
    let db_handle = db(state)?;
    let providers = store::providers::list_providers(db_handle)?;
    let mut candidate_rows: Vec<CandidateRow> = Vec::new();

    for provider in &providers {
        let accounts = store::providers::list_accounts(db_handle, &provider.id)?;
        let enabled_accounts: Vec<_> = accounts.iter().filter(|a| a.enabled).collect();
        if enabled_accounts.is_empty() {
            continue;
        }
        let overrides = store::providers::model_route_overrides(db_handle, &provider.id)?;
        for model in store::providers::list_catalog_models(db_handle, &provider.id)? {
            // embedding / rerank 不是对话模型：不编译成候选（防御历史勾选）。
            if is_non_chat_model(&model.model_id) {
                continue;
            }
            // 判断模型（judge_llm / jev）由供应商页的用途标记驱动：
            // 不再要求“勾选供应用使用”，但也不进入对话候选池。
            let judge_tag = match model.purpose {
                ModelPurposeDto::Chat => None,
                ModelPurposeDto::JudgeLlm => Some("judge:llm"),
                ModelPurposeDto::Jev => Some("judge:jev"),
            };
            if judge_tag.is_none() && !model.selected {
                continue;
            }
            if model.hidden {
                continue;
            }
            // deprecated（上游已下架）不编译出新候选。
            if store::providers::is_deprecated(db_handle, &provider.id, &model.model_id)? {
                continue;
            }
            // auto-include 哨兵行不是模型。
            if model.model_id == store::providers::AUTO_INCLUDE_SENTINEL {
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
                let base_url = account
                    .base_url_override
                    .clone()
                    .unwrap_or_else(|| provider.base_url.clone());
                // JEV 走 SystemOne：候选 base_url 归一化为完整 /v1/systemone，
                // 组判断选中该模型时可直接作为 endpoint。
                let base_url = if model.purpose == ModelPurposeDto::Jev {
                    systemone_endpoint(&base_url)
                } else {
                    base_url
                };
                let mut caps = model.caps.clone();
                if let Some(tag) = judge_tag {
                    caps.push(tag.to_string());
                }
                candidate_rows.push(CandidateRow {
                    id: candidate.clone(),
                    tier: tier.clone(),
                    strength: strength.unwrap_or(50),
                    provider: provider_kind_text(provider.kind),
                    base_url,
                    model: model.model_id.clone(),
                    credential: account.credential_ref.clone(),
                    window: model
                        .context_window
                        .filter(|value| *value > 0)
                        .unwrap_or_else(|| default_window_for(&provider.id)),
                    provider_id: provider.id.clone(),
                    account_id: account.id.clone(),
                    proxy: account.proxy_url.clone(),
                    extra_headers: account
                        .extra_headers
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|header| (header.name, header.value))
                        .collect(),
                    caps,
                    enabled: provider.enabled,
                    thinking_levels: model.thinking_levels.clone(),
                    max_output: model.max_output,
                    price: model
                        .price
                        .as_ref()
                        .map(|price| tr_agent::config::PriceFile {
                            currency: price.currency.clone(),
                            input_micros_per_mtok: price.input_micros_per_mtok,
                            output_micros_per_mtok: price.output_micros_per_mtok,
                            cache_read_micros_per_mtok: price.cache_read_micros_per_mtok,
                            cache_write_micros_per_mtok: price.cache_write_micros_per_mtok,
                            request_micros: price.request_micros,
                        }),
                });
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

    // provider 类型刚变更：把仍是旧类型默认协议的候选迁到新协议，再走正常编译。
    if let Some((provider_id, old_kind, new_kind)) = protocol_migration {
        let prefix = format!("{}-", sanitize(provider_id));
        let new_protocol = provider_kind_text(new_kind).to_string();
        for candidate in &mut file.candidates {
            if candidate.id.starts_with(&prefix)
                && protocol_is_default(&candidate.provider, old_kind)
            {
                candidate.provider = new_protocol.clone();
            }
        }
    }

    // CAS 基准：当前磁盘 revision（与 GET /config 同源），不是运行 catalog 的旧 digest。
    let disk_revision = crate::config_service::disk_revision(state)?;
    let new_ids: Vec<String> = candidate_rows.iter().map(|row| row.id.clone()).collect();

    // 本次编译全量替换「上次编译写进去的候选」，用户手写的原样保留。
    // 上次产物记在表里（migration 9）：config 没有来源字段，猜 id 前缀会漏。
    // 另外按本次产物 id 去重：即使登记表丢了，同一 id 也只会留编译版本，不会触发
    // 「candidate id 重复」。
    let previous_ids: BTreeSet<String> = store::providers::compiled_candidate_ids(db_handle)?
        .into_iter()
        .collect();
    let new_id_set: BTreeSet<&str> = new_ids.iter().map(String::as_str).collect();
    // 候选级用户设置（上游模型 / 协议 / 输入模态 / 思考档 / 请求头）不在 DB 里，
    // 重新编译时必须从上一版同 id 候选继承，否则勾选/取消别的模型会把这些设置一起抹掉。
    let previous_candidates: BTreeMap<String, tr_agent::config::CandidateFile> = file
        .candidates
        .iter()
        .map(|candidate| (candidate.id.clone(), candidate.clone()))
        .collect();
    let manual_candidates: Vec<_> = file
        .candidates
        .iter()
        .filter(|candidate| {
            !previous_ids.contains(candidate.id.as_str())
                && !new_id_set.contains(candidate.id.as_str())
        })
        .cloned()
        .collect();
    file.candidates = manual_candidates;
    // 仍被组/规则引用、但已不再编译的旧候选：保留为 **enabled = false**，而不是直接删掉。
    // 直接删会让 `virtual_models.*.candidate_pool` 悬空、整份配置校验失败（编译永远失败，
    // 前端表现为"供应商页改了勾选、组页候选纹丝不动"）。停用候选不参与路由，等引用清掉后
    // 下次编译会自然移除。
    let referenced: BTreeSet<&str> = file
        .virtual_models
        .iter()
        .flat_map(|virtual_model| {
            virtual_model
                .candidate_pool
                .iter()
                .map(String::as_str)
                .chain(virtual_model.rules.iter().map(|rule| rule.prefer.as_str()))
        })
        .collect();
    let mut retained_disabled: Vec<String> = Vec::new();
    for previous in previous_candidates.values() {
        if new_id_set.contains(previous.id.as_str()) {
            continue;
        }
        if !previous_ids.contains(previous.id.as_str()) {
            continue; // 手写候选已在 manual_candidates 里
        }
        if !referenced.contains(previous.id.as_str()) {
            continue; // 没人引用：随编译一起消失
        }
        let mut kept = previous.clone();
        kept.enabled = false;
        retained_disabled.push(kept.id.clone());
        file.candidates.push(kept);
    }
    for row in &candidate_rows {
        let previous = previous_candidates.get(&row.id);
        // 请求头：账号级为底，候选级覆盖（用户手填的优先）。
        let mut extra_headers = row.extra_headers.clone();
        if let Some(previous) = previous {
            extra_headers.extend(previous.extra_headers.clone());
        }
        file.candidates.push(tr_agent::config::CandidateFile {
            id: row.id.clone(),
            tier: row.tier.clone(),
            strength: row.strength,
            provider: previous
                .map(|candidate| candidate.provider.clone())
                .unwrap_or_else(|| row.provider.clone()),
            base_url: row.base_url.clone(),
            model: previous
                .map(|candidate| candidate.model.clone())
                .unwrap_or_else(|| row.model.clone()),
            credential: Some(row.credential.clone()),
            proxy: row.proxy.clone(),
            extra_headers,
            caps: row.caps.clone(),
            window: row.window,
            input: previous
                .map(|candidate| candidate.input.clone())
                .unwrap_or_default(),
            // 候选级推理档位优先；没有则用目录（models.dev）派生的默认档位。
            thinking_levels: previous
                .map(|candidate| candidate.thinking_levels.clone())
                .filter(|levels| !levels.is_empty())
                .unwrap_or_else(|| row.thinking_levels.clone()),
            // 候选级执行开关：关闭思考随候选保留；输出上限取目录值，目录没有时沿用旧值。
            disable_thinking: previous
                .map(|candidate| candidate.disable_thinking)
                .unwrap_or(false),
            max_tokens: row
                .max_output
                .filter(|value| *value > 0)
                .or_else(|| previous.and_then(|candidate| candidate.max_tokens)),
            price: row.price.clone(),
            enabled: row.enabled,
        });
        // 配额窗口的账号/厂商映射（P5-B 管理面分组用）。
        let _ = store::quota::ensure_mapping(
            db_handle,
            &row.id,
            Some(&row.account_id),
            Some(&row.provider_id),
        );
    }

    let document_toml = toml::to_string_pretty(&file)
        .map_err(|error| AdminApiError::internal(format!("serialize config: {error}")))?;
    let candidates_total = file.candidates.len() as u32;
    let virtual_models_total = file.virtual_models.len() as u32;
    let outcome = crate::config_service::apply_locked(
        state,
        &document_toml,
        request
            .expected_revision
            .as_deref()
            .unwrap_or(&disk_revision),
        _guard,
    )?;

    // 写盘成功后才登记产物 id；失败时保留上次记录，重试仍能正确替换。
    // 保留的停用候选也登记为"编译产物"：引用清掉后下次编译能把它们移除。
    let mut registry_ids = new_ids.clone();
    registry_ids.extend(retained_disabled.iter().cloned());
    store::providers::replace_compiled_candidates(db_handle, &registry_ids)?;
    // 配额页只展示仍可路由的候选：下架/取消勾选的模型窗口一并清掉。
    let _ = store::quota::retain_candidates(db_handle, &registry_ids);

    // 候选即时生效：热替换运行时目录（持锁版本，不能嵌套 config_write_lock）。
    if let Err(error) = crate::config_service::reload_locked(state, _guard) {
        eprintln!("config compiled but runtime reload failed: {error:?}");
    }

    Ok(CompileResponse {
        revision: outcome.revision,
        previous_revision: outcome.previous_revision.unwrap_or(disk_revision),
        candidate_ids: new_ids,
        candidates_total,
        virtual_models_total,
        backup_path: outcome.backup_path,
    })
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
    proxy: Option<String>,
    extra_headers: BTreeMap<String, String>,
    /// 能力标签（目录元数据；编译后写入候选 capability.tags）。
    caps: Vec<String>,
    provider_id: String,
    account_id: String,
    /// provider 级启停投影到候选；停用后保留引用但不再派发。
    enabled: bool,
    /// 目录里手填的价格（micros / MTok）；未定价 = None。
    price: Option<tr_agent::config::PriceFile>,
    /// 目录里记录的推理档位（models.dev 派生）；作为候选默认值。
    thinking_levels: Vec<String>,
    /// 目录里记录的最大输出（models.dev / 手填）；编译为候选 `max_tokens` 上限。
    max_output: Option<u32>,
}

fn provider_kind_text(kind: ProviderKindDto) -> String {
    match kind {
        ProviderKindDto::OpenAiCompatible => "openai".to_string(),
        ProviderKindDto::Anthropic => "anthropic".to_string(),
        ProviderKindDto::Responses => "responses".to_string(),
    }
}

/// 候选 `provider` 是否等于该供应商类型的默认协议（含 `openai_compatible` 别名）。
fn protocol_is_default(protocol: &str, kind: ProviderKindDto) -> bool {
    match kind {
        ProviderKindDto::OpenAiCompatible => {
            protocol == "openai" || protocol == "openai_compatible"
        }
        ProviderKindDto::Anthropic => protocol == "anthropic",
        ProviderKindDto::Responses => protocol == "responses",
    }
}

/// 非对话模型（embedding / rerank）不进对话候选。
///
/// 上游 `/models` 目录没有声明的模态，只能按模型 id 的通行命名过滤；
/// 误判可手动把行恢复（目录页的 hidden 状态可逆）。
fn is_non_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    const NEEDLES: [&str; 56] = [
        // embedding / rerank
        "embed",
        "rerank",
        "bge-",
        "bge_",
        "gte-",
        "e5-",
        "bce-",
        "m3e",
        "text2vec",
        "voyage-",
        "sentence-transformers",
        // 语音（TTS / ASR / 实时语音）：网关只处理文本对话
        "cosyvoice",
        "sensevoice",
        "funaudiollm",
        "whisper",
        "paraformer",
        "sambert",
        "chattts",
        "fish-audio",
        "step-audio",
        "qwen-audio",
        "glm-4-voice",
        "-tts",
        "tts-",
        "text-to-speech",
        "speech-",
        "speech_",
        "-audio",
        "audio-",
        "realtime",
        // 图像 / 视频生成
        "kolors",
        "stable-diffusion",
        "stable_diffusion",
        "sdxl",
        "flux",
        "imagen",
        "seedream",
        "seededit",
        "cogview",
        "dall-e",
        "gpt-image",
        "wanx",
        "ideogram",
        "recraft",
        "kandinsky",
        "midjourney",
        "image",
        "video",
        "veo-",
        "kling",
        "hailuo",
        "t2v",
        "i2v",
        "vidu",
        "cogvideo",
        "seedance",
    ];
    NEEDLES.iter().any(|needle| id.contains(needle))
}

/// JEV 候选的 base_url 归一化为完整 SystemOne 路径。
///
/// 供应商页通常填 OpenAI 兼容 base（`https://host/v1`）；JEV 的 SystemOne
/// 端点在同源下是 `/v1/systemone`。已填完整路径时保持不变。
fn systemone_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1/systemone") {
        return trimmed.to_string();
    }
    if trimmed.ends_with("/v1") {
        return format!("{trimmed}/systemone");
    }
    format!("{trimmed}/v1/systemone")
}

fn default_window_for(provider_id: &str) -> u32 {
    provider_templates::find_template(provider_id)
        .and_then(|t| t.default_window)
        .unwrap_or(128_000)
}

#[cfg(test)]
mod wildcard_tests {
    use super::wildcard_match;

    #[test]
    fn wildcard_semantics() {
        assert!(wildcard_match("gpt-*", "gpt-4o"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("qwen*instruct", "qwen2.5-instruct"));
        assert!(!wildcard_match("gpt-*", "claude-3"));
        assert!(!wildcard_match("gpt-", "gpt"));
    }
}

#[cfg(test)]
mod non_chat_model_tests {
    use super::is_non_chat_model;

    #[test]
    fn embedding_and_rerank_are_filtered() {
        for id in [
            "BAAI/bge-large-en-v1.5",
            "BAAI/bge-m3",
            "BAAI/bge-reranker-v2-m3",
            "text-embedding-3-small",
            "text-embedding-ada-002",
            "embed-english-v3.0",
            "rerank-v3.5",
            "gte-large",
            "e5-mistral-7b-instruct",
            "nomic-embed-text",
            "netease-youdao/bce-embedding-base_v1",
            "Pro/BAAI/bge-m3",
            "voyage-3-large",
            "text2vec-large-chinese",
            "gte-Qwen2-7B-instruct",
        ] {
            assert!(is_non_chat_model(id), "{id} 应被判定为非对话模型");
        }
    }

    #[test]
    fn chat_models_are_kept() {
        for id in [
            "gpt-4o",
            "gpt-5-codex",
            "claude-sonnet-4-5",
            "deepseek-v3",
            "qwen3-coder-plus",
            "glm-5",
            "kimi-k2",
            "gemini-2.5-pro",
        ] {
            assert!(!is_non_chat_model(id), "{id} 不应被过滤");
        }
    }

    #[test]
    fn filters_audio_and_image_families_by_name() {
        for id in [
            "FunAudioLLM/CosyVoice2-0.5B",
            "FunAudioLLM/SenseVoiceSmall",
            "Kwai-Kolors/Kolors",
            "black-forest-labs/FLUX.1-schnell",
            "openai/whisper-large-v3",
            "gpt-4o-audio-preview",
            "Qwen/Qwen-Image-Edit",
            "Tongyi-MAI/Z-Image-Turbo",
            "Wan-AI/Wan2.1-T2V-14B",
            "BAAI/bge-m3",
        ] {
            assert!(is_non_chat_model(id), "should filter: {id}");
        }
        for id in [
            "Qwen/Qwen3.6-27B",
            "deepseek-ai/DeepSeek-V3.1-Terminus",
            "gpt-4o",
            "Pro/moonshotai/Kimi-K2",
        ] {
            assert!(!is_non_chat_model(id), "should keep: {id}");
        }
    }
}
