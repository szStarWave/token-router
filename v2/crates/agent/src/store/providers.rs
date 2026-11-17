//! `providers` / `accounts` / `catalog_models` 仓储（v3 P5-A）。
//!
//! 规则：
//! - 明文 key **不入库**：只存 `credential_ref`（`file:credentials/<id>.key`）与指纹后 4 位；
//! - 目录表按 `(provider_id, model_id)` 主键 upsert；
//! - 删除 provider 级联删除账号与目录（外键）。

use rusqlite::{OptionalExtension, params};
use tr_contracts::{
    AccountInfo, CatalogModel, PriceDto, ProviderInfo, ProviderKindDto, ProviderStatus,
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
    base_url: &str,
    catalog_url: Option<&str>,
) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE providers SET display_name = ?2, base_url = ?3, catalog_url = ?4 WHERE id = ?1",
        params![id, display_name, base_url, catalog_url],
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
        created_at: row.get("created_at")?,
    })
}

const PROVIDER_SELECT: &str = "SELECT p.id, p.display_name, p.kind, p.base_url, p.catalog_url, p.status,
        p.catalog_refreshed_at, p.catalog_error, p.created_at,
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
        enabled: row.get::<_, i64>("enabled")? != 0,
        verified_at: row.get("verified_at")?,
        verify_status: row.get("verify_status")?,
        created_at: row.get("created_at")?,
    })
}

const ACCOUNT_SELECT: &str =
    "SELECT id, provider_id, label, credential_ref, fingerprint, proxy_url,
    base_url_override, enabled, verified_at, verify_status, created_at FROM accounts";

pub fn insert_account(db: &Db, account: &AccountInfo) -> Result<(), StoreError> {
    db.write().execute(
        "INSERT INTO accounts (id, provider_id, label, credential_ref, fingerprint, proxy_url,
            base_url_override, enabled, verified_at, verify_status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            account.id,
            account.provider_id,
            account.label,
            account.credential_ref,
            account.fingerprint,
            account.proxy_url,
            account.base_url_override,
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
            base_url_override = ?6, enabled = ?7 WHERE id = ?1",
        params![
            account.id,
            account.label,
            account.credential_ref,
            account.fingerprint,
            account.proxy_url,
            account.base_url_override,
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
    Ok(CatalogModel {
        model_id: row.get("model_id")?,
        display_name: row.get("display_name")?,
        selected: row.get::<_, i64>("selected")? != 0,
        candidate_id: None, // 由调用方按账号展开填充
        context_window: row.get("context_window")?,
        max_output: row.get("max_output")?,
        price: price_json.and_then(|text| serde_json::from_str(&text).ok()),
        source: row.get("source")?,
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
        tier_override: Option<String>,
        strength: Option<u8>,
    }
    let mut preserved: std::collections::BTreeMap<String, Preserved> =
        std::collections::BTreeMap::new();
    {
        let mut statement = tx.prepare(
            "SELECT model_id, selected, tier_override, strength FROM catalog_models
              WHERE provider_id = ?1",
        )?;
        let rows = statement
            .query_map(params![provider_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?.map(|value| value as u8),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (model_id, selected, tier_override, strength) in rows {
            preserved.insert(
                model_id,
                Preserved {
                    selected,
                    tier_override,
                    strength,
                },
            );
        }
    }
    tx.execute(
        "DELETE FROM catalog_models WHERE provider_id = ?1",
        params![provider_id],
    )?;
    for model in models {
        let previous = preserved.get(&model.model_id);
        let was_selected = previous.is_some_and(|p| p.selected);
        let tier_override = previous.and_then(|p| p.tier_override.clone());
        let strength = previous.and_then(|p| p.strength);
        tx.execute(
            "INSERT INTO catalog_models (provider_id, model_id, display_name, selected, context_window, max_output, price_json, source, tier_override, strength)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                provider_id,
                model.model_id,
                model.display_name,
                (model.selected || was_selected) as i64,
                model.context_window,
                model.max_output,
                model
                    .price
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                model.source,
                tier_override,
                strength,
            ],
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

pub fn list_catalog_models(db: &Db, provider_id: &str) -> Result<Vec<CatalogModel>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT model_id, display_name, selected, context_window, max_output, price_json, source
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

        update_provider(&db, "p1", "改名", "https://new.example.com/v1", None).unwrap();
        let updated = find_provider(&db, "p1").unwrap().unwrap();
        assert_eq!(updated.base_url, "https://new.example.com/v1");

        // 不存在的 provider
        assert!(find_provider(&db, "nope").unwrap().is_none());
        assert!(!update_provider(&db, "nope", "x", "u", None).unwrap());

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
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
            },
            CatalogModel {
                model_id: "m2".into(),
                display_name: "M2".into(),
                selected: false,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
            },
        ];
        upsert_catalog_models(&db, "p1", &rows).unwrap();
        set_model_selected(&db, "p1", "m1", true, Some("local"), Some(20)).unwrap();

        // 刷新（m1 保持勾选；新模型 m3 进来）
        let refreshed = vec![
            CatalogModel {
                model_id: "m1".into(),
                display_name: "M1".into(),
                selected: false,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
            },
            CatalogModel {
                model_id: "m3".into(),
                display_name: "M3".into(),
                selected: false,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".into(),
            },
        ];
        upsert_catalog_models(&db, "p1", &refreshed).unwrap();
        let models = list_catalog_models(&db, "p1").unwrap();
        assert_eq!(models.len(), 2);
        let m1 = models.iter().find(|m| m.model_id == "m1").unwrap();
        assert!(m1.selected, "刷新必须保留勾选");

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
    fn migration_6_applied() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), 6);
    }
}
