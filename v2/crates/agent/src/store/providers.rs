//! `providers` / `accounts` / `catalog_models` 仓储（v3 P5-A）。
//!
//! 规则：
//! - 明文 key **不入库**：只存 `credential_ref`（`file:credentials/<id>.key`）与指纹后 4 位；
//! - 目录表按 `(provider_id, model_id)` 主键 upsert；
//! - 删除 provider 级联删除账号与目录（外键）。

use rusqlite::{OptionalExtension, params};
use tr_contracts::{
    AccountInfo, CatalogModel, ModelPurposeDto, PriceDto, ProviderInfo, ProviderKindDto,
    ProviderStatus,
};

use crate::store::{Db, StoreError};

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn kind_from(text: &str) -> ProviderKindDto {
    match text {
        "anthropic" => ProviderKindDto::Anthropic,
        "responses" => ProviderKindDto::Responses,
        _ => ProviderKindDto::OpenAiCompatible,
    }
}

fn kind_text(kind: ProviderKindDto) -> &'static str {
    match kind {
        ProviderKindDto::OpenAiCompatible => "openai_compatible",
        ProviderKindDto::Anthropic => "anthropic",
        ProviderKindDto::Responses => "responses",
    }
}

fn status_from(text: &str) -> ProviderStatus {
    match text {
        "awaiting_catalog" => ProviderStatus::AwaitingCatalog,
        "ready" => ProviderStatus::Ready,
        "catalog_error" => ProviderStatus::CatalogError,
        _ => ProviderStatus::NotConfigured,
    }
}

fn purpose_from(text: &str) -> ModelPurposeDto {
    match text {
        "judge_llm" => ModelPurposeDto::JudgeLlm,
        "jev" => ModelPurposeDto::Jev,
        _ => ModelPurposeDto::Chat,
    }
}

fn purpose_text(purpose: ModelPurposeDto) -> &'static str {
    match purpose {
        ModelPurposeDto::Chat => "chat",
        ModelPurposeDto::JudgeLlm => "judge_llm",
        ModelPurposeDto::Jev => "jev",
    }
}

#[allow(dead_code)] // provider 状态写入目前只有 set_catalog_status 一条路径
fn status_text(status: ProviderStatus) -> &'static str {
    match status {
        ProviderStatus::NotConfigured => "not_configured",
        ProviderStatus::AwaitingCatalog => "awaiting_catalog",
        ProviderStatus::Ready => "ready",
        ProviderStatus::CatalogError => "catalog_error",
    }
}

// ── providers ────────────────────────────────────────────────────────────

/// 插入 provider（已存在则报错；调用方先 `find_provider` 判重）。
pub fn insert_provider(
    db: &Db,
    id: &str,
    display_name: &str,
    kind: ProviderKindDto,
    base_url: &str,
    catalog_url: Option<&str>,
    template_id: Option<&str>,
) -> Result<(), StoreError> {
    db.write().execute(
        "INSERT INTO providers (id, display_name, kind, base_url, catalog_url, template_id, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'not_configured', ?7)",
        params![
            id,
            display_name,
            kind_text(kind),
            base_url,
            catalog_url,
            template_id,
            now_millis()
        ],
    )?;
    Ok(())
}

/// 更新 provider 的可编辑字段。
pub fn update_provider(
    db: &Db,
    id: &str,
    display_name: &str,
    kind: ProviderKindDto,
    base_url: &str,
    catalog_url: Option<&str>,
) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE providers SET display_name = ?2, kind = ?3, base_url = ?4, catalog_url = ?5 WHERE id = ?1",
        params![id, display_name, kind_text(kind), base_url, catalog_url],
    )?;
    Ok(changed > 0)
}

/// provider 级启停；停用不删除账号/目录，下次编译把其候选标为 disabled。
pub fn set_provider_enabled(db: &Db, id: &str, enabled: bool) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE providers SET enabled = ?2 WHERE id = ?1",
        params![id, enabled as i64],
    )?;
    Ok(changed > 0)
}

pub fn delete_provider(db: &Db, id: &str) -> Result<bool, StoreError> {
    let changed = db
        .write()
        .execute("DELETE FROM providers WHERE id = ?1", params![id])?;
    Ok(changed > 0)
}

fn provider_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderInfo> {
    Ok(ProviderInfo {
        id: row.get("id")?,
        display_name: row.get("display_name")?,
        kind: kind_from(&row.get::<_, String>("kind")?),
        base_url: row.get("base_url")?,
        catalog_url: row.get("catalog_url")?,
        status: status_from(&row.get::<_, String>("status")?),
        catalog_refreshed_at: row.get("catalog_refreshed_at")?,
        catalog_error: row.get("catalog_error")?,
        account_count: row.get("account_count")?,
        candidate_count: row.get("candidate_count")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        created_at: row.get("created_at")?,
    })
}

const PROVIDER_SELECT: &str = "SELECT p.id, p.display_name, p.kind, p.base_url, p.catalog_url, p.status,
        p.catalog_refreshed_at, p.catalog_error, p.enabled, p.created_at,
        (SELECT COUNT(*) FROM accounts a WHERE a.provider_id = p.id AND a.enabled = 1) AS account_count,
        (SELECT COUNT(*) FROM catalog_models c WHERE c.provider_id = p.id AND c.selected = 1) AS candidate_count
    FROM providers p";

/// 全部 provider（按创建时间）。
pub fn list_providers(db: &Db) -> Result<Vec<ProviderInfo>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(&format!("{PROVIDER_SELECT} ORDER BY p.created_at"))?;
    let items = statement
        .query_map([], provider_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

pub fn find_provider(db: &Db, id: &str) -> Result<Option<ProviderInfo>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(&format!("{PROVIDER_SELECT} WHERE p.id = ?1"))?;
    statement
        .query_row(params![id], provider_from_row)
        .optional()
        .map_err(Into::into)
}

/// provider 创建时用的模板 id（判断 usage API 适配器用）；自定义/导入为 `None`。
pub fn find_provider_template_id(db: &Db, id: &str) -> Result<Option<String>, StoreError> {
    let connection = db.read();
    connection
        .query_row(
            "SELECT template_id FROM providers WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|row| row.flatten())
        .map_err(Into::into)
}

/// 更新目录刷新状态（成功 / 失败都写）。
pub fn set_catalog_status(
    db: &Db,
    id: &str,
    ok: bool,
    error: Option<&str>,
) -> Result<(), StoreError> {
    let status = if ok { "ready" } else { "catalog_error" };
    db.write().execute(
        "UPDATE providers SET status = ?2, catalog_refreshed_at = ?3, catalog_error = ?4 WHERE id = ?1",
        params![id, status, now_millis(), error],
    )?;
    Ok(())
}

/// 更新 provider 状态（status 为原文：not_configured / awaiting_catalog / ready / catalog_error）。
pub fn set_provider_status(db: &Db, id: &str, status: &str) -> Result<(), StoreError> {
    db.write().execute(
        "UPDATE providers SET status = ?2 WHERE id = ?1",
        params![id, status],
    )?;
    Ok(())
}

// ── accounts ─────────────────────────────────────────────────────────────

fn account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountInfo> {
    Ok(AccountInfo {
        id: row.get("id")?,
        provider_id: row.get("provider_id")?,
        label: row.get("label")?,
        credential_ref: row.get("credential_ref")?,
        fingerprint: row.get("fingerprint")?,
        proxy_url: row.get("proxy_url")?,
        base_url_override: row.get("base_url_override")?,
        extra_headers: row
            .get::<_, Option<String>>("extra_headers_json")?
            .and_then(|text| serde_json::from_str(&text).ok()),
        enabled: row.get::<_, i64>("enabled")? != 0,
        verified_at: row.get("verified_at")?,
        verify_status: row.get("verify_status")?,
        created_at: row.get("created_at")?,
    })
}

const ACCOUNT_SELECT: &str =
    "SELECT id, provider_id, label, credential_ref, fingerprint, proxy_url,
    base_url_override, extra_headers_json, enabled, verified_at, verify_status, created_at
    FROM accounts";

pub fn insert_account(db: &Db, account: &AccountInfo) -> Result<(), StoreError> {
    db.write().execute(
        "INSERT INTO accounts (id, provider_id, label, credential_ref, fingerprint, proxy_url,
            base_url_override, extra_headers_json, enabled, verified_at, verify_status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            account.id,
            account.provider_id,
            account.label,
            account.credential_ref,
            account.fingerprint,
            account.proxy_url,
            account.base_url_override,
            account
                .extra_headers
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            account.enabled as i64,
            account.verified_at,
            account.verify_status,
            account.created_at,
        ],
    )?;
    Ok(())
}

/// 更新账号；`None` 字段保持不变（SQL 里 COALESCE 语义由调用方拼装现值实现）。
pub fn update_account(db: &Db, account: &AccountInfo) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE accounts SET label = ?2, credential_ref = ?3, fingerprint = ?4, proxy_url = ?5,
            base_url_override = ?6, extra_headers_json = ?7, enabled = ?8 WHERE id = ?1",
        params![
            account.id,
            account.label,
            account.credential_ref,
            account.fingerprint,
            account.proxy_url,
            account.base_url_override,
            account
                .extra_headers
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            account.enabled as i64,
        ],
    )?;
    Ok(changed > 0)
}

pub fn delete_account(db: &Db, id: &str) -> Result<bool, StoreError> {
    let changed = db
        .write()
        .execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
    Ok(changed > 0)
}

pub fn list_accounts(db: &Db, provider_id: &str) -> Result<Vec<AccountInfo>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(&format!(
        "{ACCOUNT_SELECT} WHERE provider_id = ?1 ORDER BY created_at"
    ))?;
    let items = statement
        .query_map(params![provider_id], account_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

pub fn find_account(db: &Db, id: &str) -> Result<Option<AccountInfo>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(&format!("{ACCOUNT_SELECT} WHERE id = ?1"))?;
    statement
        .query_row(params![id], account_from_row)
        .optional()
        .map_err(Into::into)
}

/// 记录账号验证结果。
pub fn set_account_verify(db: &Db, id: &str, ok: bool, message: &str) -> Result<(), StoreError> {
    db.write().execute(
        "UPDATE accounts SET verified_at = ?2, verify_status = ?3 WHERE id = ?1",
        params![id, now_millis(), if ok { "ok" } else { message }],
    )?;
    Ok(())
}

// ── catalog_models ───────────────────────────────────────────────────────

fn model_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CatalogModel> {
    let price_json: Option<String> = row.get("price_json")?;
    let caps_json: Option<String> = row.get("caps_json")?;
    let thinking_levels_json: Option<String> = row.get("thinking_levels_json")?;
    Ok(CatalogModel {
        model_id: row.get("model_id")?,
        display_name: row.get("display_name")?,
        selected: row.get::<_, i64>("selected")? != 0,
        purpose: purpose_from(&row.get::<_, String>("purpose")?),
        caps: caps_json
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default(),
        thinking_levels: thinking_levels_json
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default(),
        candidate_id: None, // 由调用方按账号展开填充
        context_window: row.get("context_window")?,
        max_output: row.get("max_output")?,
        price: price_json.and_then(|text| serde_json::from_str(&text).ok()),
        source: row.get("source")?,
        hidden: row.get::<_, i64>("hidden")? != 0,
    })
}

/// 覆盖刷新目录（保留已勾选模型的 selected 标记）。
pub fn upsert_catalog_models(
    db: &Db,
    provider_id: &str,
    models: &[CatalogModel],
) -> Result<(), StoreError> {
    let mut connection = db.write();
    let tx = connection.transaction()?;
    // 先取已勾选与覆盖信息（删表后无法回查）。
    struct Preserved {
        selected: bool,
        purpose: String,
        caps_json: Option<String>,
        tier_override: Option<String>,
        strength: Option<u8>,
        auto_include_pattern: Option<String>,
        hidden: bool,
        context_window: Option<u32>,
        max_output: Option<u32>,
        price_json: Option<String>,
        thinking_levels_json: Option<String>,
    }
    let mut preserved: std::collections::BTreeMap<String, Preserved> =
        std::collections::BTreeMap::new();
    {
        let mut statement = tx.prepare(
            "SELECT model_id, selected, purpose, caps_json, tier_override, strength, auto_include_pattern, hidden, context_window, max_output, price_json, thinking_levels_json
              FROM catalog_models WHERE provider_id = ?1",
        )?;
        let rows = statement
            .query_map(params![provider_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?.map(|value| value as u8),
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)? != 0,
                    row.get::<_, Option<i64>>(8)?.map(|value| value as u32),
                    row.get::<_, Option<i64>>(9)?.map(|value| value as u32),
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (
            model_id,
            selected,
            purpose,
            caps_json,
            tier_override,
            strength,
            auto_include_pattern,
            hidden,
            context_window,
            max_output,
            price_json,
            thinking_levels_json,
        ) in rows
        {
            preserved.insert(
                model_id,
                Preserved {
                    selected,
                    purpose,
                    caps_json,
                    tier_override,
                    strength,
                    auto_include_pattern,
                    hidden,
                    context_window,
                    max_output,
                    price_json,
                    thinking_levels_json,
                },
            );
        }
    }
    // provider 级 auto-include 通配符存在哨兵行上；哨兵不在上游 fresh 目录里，
    // 下面的 DELETE 会把它一起删掉，需要原样恢复。
    let sentinel_pattern = preserved
        .get(AUTO_INCLUDE_SENTINEL)
        .and_then(|preserved| preserved.auto_include_pattern.clone());
    tx.execute(
        "DELETE FROM catalog_models WHERE provider_id = ?1",
        params![provider_id],
    )?;
    for model in models {
        let previous = preserved.get(&model.model_id);
        let was_selected = previous.is_some_and(|p| p.selected);
        let purpose = previous
            .map(|p| p.purpose.clone())
            .unwrap_or_else(|| purpose_text(model.purpose).to_string());
        let caps_json = previous.and_then(|p| p.caps_json.clone()).or_else(|| {
            if model.caps.is_empty() {
                None
            } else {
                serde_json::to_string(&model.caps).ok()
            }
        });
        let tier_override = previous.and_then(|p| p.tier_override.clone());
        let strength = previous.and_then(|p| p.strength);
        let auto_include_pattern = previous.and_then(|p| p.auto_include_pattern.clone());
        let hidden = previous.is_some_and(|p| p.hidden);
        // 用户手填的模型级参数（窗口 / 最大输出 / 价格）优先于上游目录元数据：
        // 刷新目录不应抹掉手动设置的值。
        let context_window = previous
            .and_then(|p| p.context_window)
            .or(model.context_window);
        let max_output = previous.and_then(|p| p.max_output).or(model.max_output);
        let price_json = match previous.and_then(|p| p.price_json.clone()) {
            Some(existing) => Some(existing),
            None => model
                .price
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        };
        // 目录刷新不覆盖已自动填充的推理档位。
        let thinking_levels_json = match previous.and_then(|p| p.thinking_levels_json.clone()) {
            Some(existing) => Some(existing),
            None if model.thinking_levels.is_empty() => None,
            None => Some(serde_json::to_string(&model.thinking_levels)?),
        };
        tx.execute(
            "INSERT INTO catalog_models (provider_id, model_id, display_name, selected, purpose, caps_json, context_window, max_output, price_json, source, tier_override, strength, auto_include_pattern, hidden, thinking_levels_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                provider_id,
                model.model_id,
                model.display_name,
                (model.selected || was_selected) as i64,
                purpose,
                caps_json,
                context_window,
                max_output,
                price_json,
                model.source,
                tier_override,
                strength,
                auto_include_pattern,
                hidden as i64,
                thinking_levels_json,
            ],
        )?;
    }
    if let Some(pattern) = sentinel_pattern {
        // 与 set_provider_auto_include 相同的 upsert 语义：极小概率上游目录
        // 恰好有与哨兵同名的 model_id 时走 DO UPDATE，不让整个刷新回滚。
        tx.execute(
            "INSERT INTO catalog_models (provider_id, model_id, display_name, selected, source, auto_include_pattern)
             VALUES (?1, ?2, '', 0, 'manual', ?3)
             ON CONFLICT(provider_id, model_id) DO UPDATE SET auto_include_pattern = excluded.auto_include_pattern",
            params![provider_id, AUTO_INCLUDE_SENTINEL, pattern],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// 手工目录：条目不存在时隐式创建（本地推理等无在线目录场景）。返回是否新建。
pub fn upsert_manual_model(
    db: &Db,
    provider_id: &str,
    model_id: &str,
    selected: bool,
) -> Result<bool, StoreError> {
    let inserted = db.write().execute(
        "INSERT OR IGNORE INTO catalog_models (provider_id, model_id, display_name, selected, source)
         VALUES (?1, ?2, ?2, ?3, 'manual')",
        params![provider_id, model_id, selected as i64],
    )?;
    Ok(inserted > 0)
}

/// 目录条目是否存在。
pub fn catalog_model_exists(
    db: &Db,
    provider_id: &str,
    model_id: &str,
) -> Result<bool, StoreError> {
    let exists = db.read().query_row(
        "SELECT 1 FROM catalog_models WHERE provider_id = ?1 AND model_id = ?2",
        params![provider_id, model_id],
        |_| Ok(()),
    );
    match exists {
        Ok(()) => Ok(true),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// 勾选 / 取消勾选模型。
pub fn set_model_selected(
    db: &Db,
    provider_id: &str,
    model_id: &str,
    selected: bool,
    tier_override: Option<&str>,
    strength: Option<u8>,
) -> Result<bool, StoreError> {
    let mut connection = db.write();
    let tx = connection.transaction()?;
    // 勾选信息拆两处：selected 在 catalog_models；tier/strength 覆盖也在同一行。
    let changed = tx.execute(
        "UPDATE catalog_models SET selected = ?3,
            tier_override = COALESCE(?4, tier_override),
            strength = COALESCE(?5, strength)
         WHERE provider_id = ?1 AND model_id = ?2",
        params![
            provider_id,
            model_id,
            selected as i64,
            tier_override,
            strength
        ],
    )?;
    tx.commit()?;
    Ok(changed > 0)
}

/// 目录模型的可编辑元数据补丁（`None` = 该字段保持不变）。
///
/// 与 `selected` 解耦：用途只是选择器分组元数据，能力标签在编译时进入候选 tags；
/// 窗口 / 最大输出 / 价格是模型级参数（编译时写进候选）。
/// `Some(0)` = 清除该数值（回到未设置）；价格为全 0 时同样清除。
#[derive(Debug, Clone, Default)]
pub struct ModelMetadataPatch {
    pub purpose: Option<ModelPurposeDto>,
    pub caps: Option<Vec<String>>,
    pub context_window: Option<u32>,
    pub max_output: Option<u32>,
    pub price: Option<PriceDto>,
}

pub fn update_model_metadata(
    db: &Db,
    provider_id: &str,
    model_id: &str,
    patch: &ModelMetadataPatch,
) -> Result<bool, StoreError> {
    let mut connection = db.write();
    let tx = connection.transaction()?;
    let mut changed = false;
    if let Some(purpose) = patch.purpose {
        changed |= tx.execute(
            "UPDATE catalog_models SET purpose = ?3 WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, model_id, purpose_text(purpose)],
        )? > 0;
    }
    if let Some(caps) = &patch.caps {
        changed |= tx.execute(
            "UPDATE catalog_models SET caps_json = ?3 WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, model_id, serde_json::to_string(caps)?],
        )? > 0;
    }
    if let Some(window) = patch.context_window {
        changed |= tx.execute(
            "UPDATE catalog_models SET context_window = ?3 WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, model_id, window],
        )? > 0;
    }
    if let Some(max_output) = patch.max_output {
        changed |= tx.execute(
            "UPDATE catalog_models SET max_output = ?3 WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, model_id, max_output],
        )? > 0;
    }
    if let Some(price) = &patch.price {
        // 全 0 视为「清除价格」：价格未定价时按内核 unknown_price_policy 处理。
        let is_empty = price.input_micros_per_mtok == 0
            && price.output_micros_per_mtok == 0
            && price.cache_read_micros_per_mtok.unwrap_or(0) == 0
            && price.cache_write_micros_per_mtok.unwrap_or(0) == 0
            && price.request_micros == 0;
        let payload = if is_empty {
            None
        } else {
            Some(serde_json::to_string(price)?)
        };
        changed |= tx.execute(
            "UPDATE catalog_models SET price_json = ?3 WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, model_id, payload],
        )? > 0;
    }
    tx.commit()?;
    Ok(changed)
}

/// 标记目录元数据的来源（`catalog` / `models.dev` / `manual`）。
pub fn set_model_source(
    db: &Db,
    provider_id: &str,
    model_id: &str,
    source: &str,
) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE catalog_models SET source = ?3 WHERE provider_id = ?1 AND model_id = ?2",
        params![provider_id, model_id, source],
    )?;
    Ok(changed > 0)
}

/// 写入目录模型的推理档位（models.dev 自动填充或手工修正）。
pub fn set_model_thinking_levels(
    db: &Db,
    provider_id: &str,
    model_id: &str,
    levels: &[String],
) -> Result<bool, StoreError> {
    let payload = if levels.is_empty() {
        None
    } else {
        Some(serde_json::to_string(levels)?)
    };
    let changed = db.write().execute(
        "UPDATE catalog_models SET thinking_levels_json = ?3 WHERE provider_id = ?1 AND model_id = ?2",
        params![provider_id, model_id, payload],
    )?;
    Ok(changed > 0)
}

/// 持久隐藏/恢复目录条目；删除同时取消勾选。
pub fn set_model_hidden(
    db: &Db,
    provider_id: &str,
    model_id: &str,
    hidden: bool,
) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE catalog_models SET hidden = ?3, selected = CASE WHEN ?3 = 1 THEN 0 ELSE selected END
         WHERE provider_id = ?1 AND model_id = ?2",
        params![provider_id, model_id, hidden as i64],
    )?;
    Ok(changed > 0)
}

pub fn list_catalog_models(db: &Db, provider_id: &str) -> Result<Vec<CatalogModel>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT model_id, display_name, selected, purpose, caps_json, context_window, max_output, price_json, source, hidden, thinking_levels_json
           FROM catalog_models WHERE provider_id = ?1 ORDER BY model_id",
    )?;
    let items = statement
        .query_map(params![provider_id], model_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

/// 勾选模型的路由覆盖：model_id → (tier_override, strength)。
pub type ModelRouteOverrides = std::collections::BTreeMap<String, (Option<String>, Option<u8>)>;

/// 读取勾选模型的路由覆盖（tier/strength）。
pub fn model_route_overrides(
    db: &Db,
    provider_id: &str,
) -> Result<ModelRouteOverrides, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT model_id, tier_override, strength FROM catalog_models
          WHERE provider_id = ?1 AND selected = 1",
    )?;
    let mut map = std::collections::BTreeMap::new();
    let rows = statement
        .query_map(params![provider_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?.map(|v| v as u8),
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (model_id, tier, strength) in rows {
        map.insert(model_id, (tier, strength));
    }
    Ok(map)
}

/// price_json 解析（测试用）。
pub fn parse_price(text: &str) -> Option<PriceDto> {
    serde_json::from_str(text).ok()
}

/// 目录 auto-include 通配符列表（去重）。
pub fn auto_include_patterns(db: &Db, provider_id: &str) -> Result<Vec<String>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT DISTINCT auto_include_pattern FROM catalog_models
          WHERE provider_id = ?1 AND auto_include_pattern IS NOT NULL",
    )?;
    let items = statement
        .query_map(params![provider_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

/// 上游目录不再包含的模型标记 deprecated；重新出现的清除标记。
pub fn mark_deprecated(
    db: &Db,
    provider_id: &str,
    fresh_ids: &[String],
    now_millis: i64,
) -> Result<(), StoreError> {
    let mut connection = db.write();
    let tx = connection.transaction()?;
    // 不在 fresh 集合里的 → 标记 deprecated（保留 selected 以便恢复）。
    tx.execute(
        "UPDATE catalog_models SET deprecated = 1, deprecated_detected_at = ?2
          WHERE provider_id = ?1 AND deprecated = 0 AND model_id NOT IN (SELECT value FROM json_each(?3))",
        params![provider_id, now_millis, serde_json::to_string(fresh_ids)?],
    )?;
    // 重新出现的 → 清除。
    tx.execute(
        "UPDATE catalog_models SET deprecated = 0, deprecated_detected_at = NULL
          WHERE provider_id = ?1 AND deprecated = 1 AND model_id IN (SELECT value FROM json_each(?2))",
        params![provider_id, serde_json::to_string(fresh_ids)?],
    )?;
    tx.commit()?;
    Ok(())
}

/// 设置某条目录条目的 auto-include 通配符（manual 条目上配置）。
pub fn set_auto_include_pattern(
    db: &Db,
    provider_id: &str,
    model_id: &str,
    pattern: Option<&str>,
) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE catalog_models SET auto_include_pattern = ?3
          WHERE provider_id = ?1 AND model_id = ?2",
        params![provider_id, model_id, pattern],
    )?;
    Ok(changed > 0)
}

/// 手动清除单条模型的 deprecated 标记。
pub fn clear_deprecated(db: &Db, provider_id: &str, model_id: &str) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE catalog_models SET deprecated = 0, deprecated_detected_at = NULL
          WHERE provider_id = ?1 AND model_id = ?2",
        params![provider_id, model_id],
    )?;
    Ok(changed > 0)
}

/// 读取单条模型的 deprecated 标记。
pub fn is_deprecated(db: &Db, provider_id: &str, model_id: &str) -> Result<bool, StoreError> {
    let connection = db.read();
    let flag: Option<i64> = connection
        .query_row(
            "SELECT deprecated FROM catalog_models WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, model_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(flag.unwrap_or(0) != 0)
}

/// 上次编译写进 config 的候选 id 集合（migration 9）。
///
/// 编译需要区分「编译产物」与「用户手写候选」：config 里没有来源字段，
/// 只能记住上次写了哪些 id，本次全量替换这批、其余原样保留。
pub fn compiled_candidate_ids(db: &Db) -> Result<Vec<String>, StoreError> {
    let connection = db.read();
    let mut statement =
        connection.prepare("SELECT candidate_id FROM compiled_candidates ORDER BY candidate_id")?;
    let items = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

/// 覆盖记录本次编译产物（空集合 = 清空，表示不再有编译候选）。
pub fn replace_compiled_candidates(db: &Db, ids: &[String]) -> Result<(), StoreError> {
    let mut connection = db.write();
    let tx = connection.transaction()?;
    tx.execute("DELETE FROM compiled_candidates", [])?;
    let now = now_millis();
    for id in ids {
        tx.execute(
            "INSERT INTO compiled_candidates (candidate_id, compiled_at) VALUES (?1, ?2)",
            params![id, now],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_crud_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "DeepSeek 实例",
            ProviderKindDto::OpenAiCompatible,
            "https://api.deepseek.com/v1",
            None,
            Some("deepseek"),
        )
        .unwrap();
        // 重复 id 报错
        assert!(
            insert_provider(
                &db,
                "p1",
                "x",
                ProviderKindDto::OpenAiCompatible,
                "u",
                None,
                None
            )
            .is_err()
        );

        let found = find_provider(&db, "p1").unwrap().unwrap();
        assert_eq!(found.display_name, "DeepSeek 实例");
        assert_eq!(found.status, ProviderStatus::NotConfigured);
        assert_eq!(found.account_count, 0);
        assert!(found.enabled);

        set_provider_enabled(&db, "p1", false).unwrap();
        assert!(!find_provider(&db, "p1").unwrap().unwrap().enabled);
        set_provider_enabled(&db, "p1", true).unwrap();
        assert!(find_provider(&db, "p1").unwrap().unwrap().enabled);

        update_provider(
            &db,
            "p1",
            "改名",
            ProviderKindDto::OpenAiCompatible,
            "https://new.example.com/v1",
            None,
        )
        .unwrap();
        let updated = find_provider(&db, "p1").unwrap().unwrap();
        assert_eq!(updated.base_url, "https://new.example.com/v1");

        // 不存在的 provider
        assert!(find_provider(&db, "nope").unwrap().is_none());
        assert!(
            !update_provider(
                &db,
                "nope",
                "x",
                ProviderKindDto::OpenAiCompatible,
                "u",
                None
            )
            .unwrap()
        );

        delete_provider(&db, "p1").unwrap();
        assert!(find_provider(&db, "p1").unwrap().is_none());
    }

    #[test]
    fn account_crud_and_counts() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        let account = AccountInfo {
            id: "a1".into(),
            provider_id: "p1".into(),
            label: "主账号".into(),
            credential_ref: "file:credentials/a1.key".into(),
            fingerprint: "ab12".into(),
            proxy_url: None,
            base_url_override: None,
            extra_headers: None,
            enabled: true,
            verified_at: None,
            verify_status: None,
            created_at: 1,
        };
        insert_account(&db, &account).unwrap();
        assert_eq!(find_provider(&db, "p1").unwrap().unwrap().account_count, 1);

        // 验证结果写入
        set_account_verify(&db, "a1", true, "ok (3 models)").unwrap();
        let verified = find_account(&db, "a1").unwrap().unwrap();
        assert_eq!(verified.verify_status.as_deref(), Some("ok"));
        assert!(verified.verified_at.is_some());

        // 禁用 → account_count 排除
        let mut disabled = account.clone();
        disabled.enabled = false;
        update_account(&db, &disabled).unwrap();
        assert_eq!(find_provider(&db, "p1").unwrap().unwrap().account_count, 0);

        // 删除 provider 级联删账号
        delete_provider(&db, "p1").unwrap();
        assert!(find_account(&db, "a1").unwrap().is_none());
    }

    #[test]
    fn catalog_selection_preserved_on_refresh() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        let rows = vec![
            CatalogModel {
                model_id: "m1".into(),
                display_name: "M1".into(),
                selected: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
                hidden: false,
            },
            CatalogModel {
                model_id: "m2".into(),
                display_name: "M2".into(),
                selected: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
                hidden: false,
            },
        ];
        upsert_catalog_models(&db, "p1", &rows).unwrap();
        set_model_selected(&db, "p1", "m1", true, Some("local"), Some(20)).unwrap();
        update_model_metadata(
            &db,
            "p1",
            "m1",
            &ModelMetadataPatch {
                purpose: Some(ModelPurposeDto::JudgeLlm),
                caps: Some(vec!["代码".to_string(), "长上下文".to_string()]),
                context_window: Some(131_072),
                max_output: Some(8_192),
                price: Some(PriceDto {
                    currency: "USD".to_string(),
                    input_micros_per_mtok: 150_000,
                    output_micros_per_mtok: 600_000,
                    cache_read_micros_per_mtok: Some(3_000),
                    cache_write_micros_per_mtok: None,
                    request_micros: 0,
                }),
            },
        )
        .unwrap();

        // 刷新（m1 保持勾选；新模型 m3 进来）
        let refreshed = vec![
            CatalogModel {
                model_id: "m1".into(),
                display_name: "M1".into(),
                selected: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
                hidden: false,
            },
            CatalogModel {
                model_id: "m3".into(),
                display_name: "M3".into(),
                selected: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
                hidden: false,
            },
        ];
        upsert_catalog_models(&db, "p1", &refreshed).unwrap();
        let models = list_catalog_models(&db, "p1").unwrap();
        assert_eq!(models.len(), 2);
        let m1 = models.iter().find(|m| m.model_id == "m1").unwrap();
        assert!(m1.selected, "刷新必须保留勾选");
        assert_eq!(m1.purpose, ModelPurposeDto::JudgeLlm, "刷新必须保留用途");
        assert_eq!(m1.caps, vec!["代码".to_string(), "长上下文".to_string()]);

        let overrides = model_route_overrides(&db, "p1").unwrap();
        assert_eq!(
            overrides.get("m1"),
            Some(&(Some("local".to_string()), Some(20)))
        );

        // 取消勾选
        set_model_selected(&db, "p1", "m1", false, None, None).unwrap();
        assert!(model_route_overrides(&db, "p1").unwrap().is_empty());
    }

    #[test]
    fn manual_model_upsert() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        assert!(upsert_manual_model(&db, "p1", "qwen2.5", true).unwrap());
        assert!(
            !upsert_manual_model(&db, "p1", "qwen2.5", true).unwrap(),
            "第二次是幂等更新"
        );
        let models = list_catalog_models(&db, "p1").unwrap();
        assert_eq!(models.len(), 1);
        assert!(models[0].selected);
        assert_eq!(models[0].source, "manual");
    }

    #[test]
    fn hidden_model_survives_catalog_refresh() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        let row = CatalogModel {
            model_id: "m1".into(),
            display_name: "M1".into(),
            selected: false,
            purpose: ModelPurposeDto::Chat,
            caps: Vec::new(),
            thinking_levels: Vec::new(),
            candidate_id: None,
            context_window: None,
            max_output: None,
            price: None,
            source: "catalog".into(),
            hidden: false,
        };
        upsert_catalog_models(&db, "p1", std::slice::from_ref(&row)).unwrap();
        set_model_hidden(&db, "p1", "m1", true).unwrap();

        // 目录刷新会重建行，但 hidden 必须保留。
        upsert_catalog_models(&db, "p1", std::slice::from_ref(&row)).unwrap();
        let models = list_catalog_models(&db, "p1").unwrap();
        assert!(models[0].hidden);
        assert!(!models[0].selected);
    }

    #[test]
    fn migration_6_applied() {
        let db = Db::open_in_memory().unwrap();
        // M6 providers/accounts/catalog + M7 extra_headers/auto-include/deprecated
        // + M9 compiled_candidates 登记表 + M11 provider enabled + M12 用途/能力标签
        // + M15 每轮消息记录 + M16 判官调用详情。
        assert_eq!(db.schema_version().unwrap(), 16);
    }

    #[test]
    fn compiled_candidate_registry_replaces_atomically() {
        let db = Db::open_in_memory().unwrap();
        assert!(compiled_candidate_ids(&db).unwrap().is_empty());

        replace_compiled_candidates(&db, &["a".to_string(), "b".to_string()]).unwrap();
        assert_eq!(compiled_candidate_ids(&db).unwrap(), vec!["a", "b"]);

        // 第二次编译换了一批 id：旧记录必须整批消失，否则下次编译还会把它们当手写候选。
        replace_compiled_candidates(&db, &["c".to_string()]).unwrap();
        assert_eq!(compiled_candidate_ids(&db).unwrap(), vec!["c"]);

        replace_compiled_candidates(&db, &[]).unwrap();
        assert!(compiled_candidate_ids(&db).unwrap().is_empty());
    }
}

/// provider 级 auto-include 通配符（存在哨兵行上，不参与编译）。
pub const AUTO_INCLUDE_SENTINEL: &str = "(auto-include)";

/// 写入/清除 provider 级 auto-include 通配符。
pub fn set_provider_auto_include(
    db: &Db,
    provider_id: &str,
    pattern: Option<&str>,
) -> Result<(), StoreError> {
    let mut connection = db.write();
    let tx = connection.transaction()?;
    match pattern.map(str::trim).filter(|pattern| !pattern.is_empty()) {
        Some(pattern) => {
            tx.execute(
                "INSERT INTO catalog_models (provider_id, model_id, display_name, selected, source, auto_include_pattern)
                 VALUES (?1, ?2, '', 0, 'manual', ?3)
                 ON CONFLICT(provider_id, model_id) DO UPDATE SET auto_include_pattern = excluded.auto_include_pattern",
                params![provider_id, AUTO_INCLUDE_SENTINEL, pattern],
            )?;
        }
        None => {
            tx.execute(
                "DELETE FROM catalog_models WHERE provider_id = ?1 AND model_id = ?2",
                params![provider_id, AUTO_INCLUDE_SENTINEL],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// 读取 provider 级 auto-include 通配符。
pub fn provider_auto_include(db: &Db, provider_id: &str) -> Result<Option<String>, StoreError> {
    let connection = db.read();
    connection
        .query_row(
            "SELECT auto_include_pattern FROM catalog_models WHERE provider_id = ?1 AND model_id = ?2",
            params![provider_id, AUTO_INCLUDE_SENTINEL],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(Into::into)
}

#[cfg(test)]
mod auto_include_tests {
    use super::*;

    #[test]
    fn provider_pattern_roundtrip_and_sentinel_excluded() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        set_provider_auto_include(&db, "p1", Some("gpt-*")).unwrap();
        assert_eq!(
            provider_auto_include(&db, "p1").unwrap().as_deref(),
            Some("gpt-*")
        );
        // 通配符能被 apply 逻辑读到。
        assert_eq!(
            auto_include_patterns(&db, "p1").unwrap(),
            vec!["gpt-*".to_string()]
        );
        // 清除。
        set_provider_auto_include(&db, "p1", None).unwrap();
        assert!(provider_auto_include(&db, "p1").unwrap().is_none());
    }

    /// 模拟 control 侧 `apply_auto_include`：读通配符并勾选命中的未选模型。
    fn apply_auto_include_like_control(db: &Db, provider_id: &str) {
        let patterns = auto_include_patterns(db, provider_id).unwrap();
        if patterns.is_empty() {
            return;
        }
        for model in list_catalog_models(db, provider_id).unwrap() {
            if model.selected {
                continue;
            }
            if patterns
                .iter()
                .any(|pattern| wildcard_match(pattern, &model.model_id))
            {
                set_model_selected(db, provider_id, &model.model_id, true, None, None).unwrap();
            }
        }
    }

    /// magpie 口径通配符（只支持 `*`），与 control::provider_service 一致。
    fn wildcard_match(pattern: &str, text: &str) -> bool {
        fn inner(p: &[u8], t: &[u8]) -> bool {
            if p.is_empty() {
                return t.is_empty();
            }
            match p[0] {
                b'*' => (0..=t.len()).any(|skip| inner(&p[1..], &t[skip..])),
                byte => !t.is_empty() && t[0] == byte && inner(&p[1..], &t[1..]),
            }
        }
        inner(pattern.as_bytes(), text.as_bytes())
    }

    fn catalog_row(id: &str) -> CatalogModel {
        CatalogModel {
            model_id: id.into(),
            display_name: id.to_uppercase(),
            selected: false,
            purpose: ModelPurposeDto::Chat,
            caps: Vec::new(),
            thinking_levels: Vec::new(),
            candidate_id: None,
            context_window: None,
            max_output: None,
            price: None,
            source: "catalog".into(),
            hidden: false,
        }
    }

    #[test]
    fn auto_include_patterns_and_sentinel_survive_refresh() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        upsert_catalog_models(&db, "p1", &[catalog_row("deepseek-chat")]).unwrap();
        // provider 级模式（哨兵行）+ 单模型模式。
        set_provider_auto_include(&db, "p1", Some("gpt-*")).unwrap();
        set_auto_include_pattern(&db, "p1", "deepseek-chat", Some("deepseek-chat")).unwrap();

        // 刷新 1：条目保留，DELETE+重 insert 后两处 pattern 都必须原样回来。
        upsert_catalog_models(&db, "p1", &[catalog_row("deepseek-chat")]).unwrap();
        assert_eq!(
            provider_auto_include(&db, "p1").unwrap().as_deref(),
            Some("gpt-*"),
            "provider 级模式（哨兵行）刷新后不得丢失"
        );
        let patterns = auto_include_patterns(&db, "p1").unwrap();
        assert!(
            patterns.contains(&"gpt-*".to_string())
                && patterns.contains(&"deepseek-chat".to_string()),
            "单模型与 provider 级模式刷新后都必须还在，实际: {patterns:?}"
        );

        // 刷新 2：新模型 gpt-4o-mini 出现，自动勾选依赖两处 pattern 都存活。
        upsert_catalog_models(
            &db,
            "p1",
            &[catalog_row("deepseek-chat"), catalog_row("gpt-4o-mini")],
        )
        .unwrap();
        // 刷新 2 后模式必须仍然全部存在。
        assert_eq!(
            provider_auto_include(&db, "p1").unwrap().as_deref(),
            Some("gpt-*"),
            "第二次刷新后 provider 级模式仍须存在"
        );
        let patterns = auto_include_patterns(&db, "p1").unwrap();
        assert!(
            patterns.contains(&"gpt-*".to_string())
                && patterns.contains(&"deepseek-chat".to_string()),
            "第二次刷新后单模型模式仍须存在，实际: {patterns:?}"
        );

        // 与 control 相同的 apply 逻辑：新模型按 provider 级模式自动勾选。
        apply_auto_include_like_control(&db, "p1");
        let models = list_catalog_models(&db, "p1").unwrap();
        let gpt = models.iter().find(|m| m.model_id == "gpt-4o-mini").unwrap();
        assert!(gpt.selected, "新出现且命中通配符的模型应被自动勾选");
        let ds = models
            .iter()
            .find(|m| m.model_id == "deepseek-chat")
            .unwrap();
        assert!(ds.selected, "命中自身单模型模式的模型应被自动勾选");
        // 注：store 层 list_catalog_models 会包含哨兵行；control 侧负责过滤
        // （provider_service 的 catalog 列表与 auto-include apply 均排除哨兵）。
    }

    #[test]
    fn deprecated_flow_unaffected_by_pattern_preservation() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        upsert_catalog_models(&db, "p1", &[catalog_row("m1"), catalog_row("m2")]).unwrap();
        set_provider_auto_include(&db, "p1", Some("m-*")).unwrap();
        set_model_selected(&db, "p1", "m1", true, None, None).unwrap();

        // m2 从上游消失 → deprecated；重新出现 → 标记清除（原逻辑不变）。
        upsert_catalog_models(&db, "p1", &[catalog_row("m1")]).unwrap();
        let fresh = vec!["m1".to_string()];
        mark_deprecated(&db, "p1", &fresh, 1000).unwrap();
        assert!(!is_deprecated(&db, "p1", "m1").unwrap());

        upsert_catalog_models(&db, "p1", &[catalog_row("m1"), catalog_row("m2")]).unwrap();
        mark_deprecated(&db, "p1", &["m1".to_string(), "m2".to_string()], 2000).unwrap();
        assert!(
            !is_deprecated(&db, "p1", "m2").unwrap(),
            "重新出现的模型 deprecated 应清除"
        );
        // 通配符不因 deprecated 流程丢失。
        assert_eq!(
            provider_auto_include(&db, "p1").unwrap().as_deref(),
            Some("m-*")
        );
    }

    #[test]
    fn thinking_levels_persist_and_survive_refresh() {
        let db = Db::open_in_memory().unwrap();
        insert_provider(
            &db,
            "p1",
            "P",
            ProviderKindDto::OpenAiCompatible,
            "u",
            None,
            None,
        )
        .unwrap();
        let mut row = catalog_row("m1");
        row.thinking_levels = vec!["low".to_string(), "high".to_string()];
        upsert_catalog_models(&db, "p1", std::slice::from_ref(&row)).unwrap();
        let models = list_catalog_models(&db, "p1").unwrap();
        assert_eq!(
            models[0].thinking_levels,
            vec!["low".to_string(), "high".to_string()]
        );

        // 目录刷新（空档位）不覆盖已填充的值。
        row.thinking_levels = Vec::new();
        upsert_catalog_models(&db, "p1", std::slice::from_ref(&row)).unwrap();
        let models = list_catalog_models(&db, "p1").unwrap();
        assert_eq!(
            models[0].thinking_levels,
            vec!["low".to_string(), "high".to_string()]
        );
    }
}
