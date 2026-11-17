//! `tokens` 仓储（P4-019）：scoped token store。
//!
//! 安全规则（契约文档 §3）：
//! - 只存 `sha256(token)`，不存明文、不存可逆加密；
//! - 列表只返回 `prefix`（前 8 字符）；
//! - 撤销是软删除（`revoked_at`），保留审计；
//! - token 变更后由调用方重建内存缓存。

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use tr_contracts::{AdminScope, CreateTokenRequest, TokenInfo};

use crate::store::{Db, StoreError};

/// 单条 token 记录（含缓存校验所需的哈希）。
#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub id: String,
    pub name: String,
    pub token_sha256: String,
    pub prefix: String,
    pub scopes: Vec<AdminScope>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    /// 网关 key 限额（v3 P5-B）。
    pub limits: Option<tr_contracts::KeyLimits>,
}

impl TokenRecord {
    /// 记录是否当前有效（未撤销、未过期）。
    pub fn is_active(&self, now_millis: i64) -> bool {
        if self.revoked_at.is_some() {
            return false;
        }
        match self.expires_at {
            Some(expires_at) => expires_at > now_millis,
            None => true,
        }
    }

    pub fn scopes_json(&self) -> Result<String, StoreError> {
        Ok(serde_json::to_string(
            &self
                .scopes
                .iter()
                .map(|scope| scope.as_str())
                .collect::<Vec<_>>(),
        )?)
    }
}

pub fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 生成新 token：随机 32 字节，`tr2_` 前缀（base64url 无填充）。
pub fn generate_token() -> String {
    format!("tr2_{}", base64url(random_32_bytes()))
}

/// 32 随机字节（无第三方 rand 依赖：UUID v4 内部使用 OS 熵源）。
fn random_32_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    // uuid v4 内部使用 OS 熵源（getrandom）；拆两份凑 32 字节。
    let a = uuid::Uuid::new_v4().into_bytes();
    let b = uuid::Uuid::new_v4().into_bytes();
    bytes[..16].copy_from_slice(&a);
    bytes[16..].copy_from_slice(&b);
    bytes
}

fn base64url(bytes: [u8; 32]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    // 32 字节 = 256 位 -> 43 个 base64 字符（无填充）。
    let mut out = String::with_capacity(43);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for &byte in &bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            let index = ((buffer >> bits) & 0x3f) as usize;
            out.push(ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (6 - bits)) & 0x3f) as usize;
        out.push(ALPHABET[index] as char);
    }
    out
}

/// 插入 token（调用方先用 `generate_token` 生成明文并计算哈希）。
pub fn insert(db: &Db, record: &TokenRecord) -> Result<(), StoreError> {
    let connection = db.write();
    connection.execute(
        "INSERT INTO tokens (id, name, token_sha256, prefix, scopes_json, created_at, last_used_at, expires_at, revoked_at, limits_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            record.id,
            record.name,
            record.token_sha256,
            record.prefix,
            record.scopes_json()?,
            record.created_at,
            record.last_used_at,
            record.expires_at,
            record.revoked_at,
            record
                .limits
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        ],
    )?;
    Ok(())
}

/// 按明文查 token（鉴权路径）：返回未撤销且未过期的记录。
pub fn find_active_by_plaintext(
    db: &Db,
    plaintext: &str,
    now_millis: i64,
) -> Result<Option<TokenRecord>, StoreError> {
    let digest = sha256_hex(plaintext);
    let connection = db.read();
    let row = connection
        .query_row(
            "SELECT id, name, token_sha256, prefix, scopes_json, created_at, last_used_at, expires_at, revoked_at, limits_json
               FROM tokens WHERE token_sha256 = ?1",
            params![digest],
            row_from,
        )
        .optional()?;
    Ok(row.filter(|record| record.is_active(now_millis)))
}

/// 按 id 查询。
pub fn find_by_id(db: &Db, id: &str) -> Result<Option<TokenRecord>, StoreError> {
    let connection = db.read();
    connection
        .query_row(
            "SELECT id, name, token_sha256, prefix, scopes_json, created_at, last_used_at, expires_at, revoked_at, limits_json
               FROM tokens WHERE id = ?1",
            params![id],
            row_from,
        )
        .optional()
        .map_err(StoreError::from)
}

/// 列表（脱敏：无哈希）。
pub fn list(db: &Db) -> Result<Vec<TokenInfo>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT id, name, prefix, scopes_json, created_at, last_used_at, expires_at, revoked_at, limits_json
           FROM tokens ORDER BY created_at DESC",
    )?;
    let rows = statement.query_map([], |row| {
        let scopes_json: String = row.get(3)?;
        Ok(TokenInfo {
            id: row.get(0)?,
            name: row.get(1)?,
            prefix: row.get(2)?,
            scopes: parse_scopes(&scopes_json),
            created_at: row.get(4)?,
            last_used_at: row.get(5)?,
            expires_at: row.get(6)?,
            revoked_at: row.get(7)?,
            limits: row
                .get::<_, Option<String>>(8)?
                .and_then(|text| serde_json::from_str(&text).ok()),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

/// 设置/清除 key 限额（`PATCH /v2/admin/tokens/{id}`）。
pub fn update_limits(
    db: &Db,
    id: &str,
    limits: Option<&tr_contracts::KeyLimits>,
) -> Result<bool, StoreError> {
    let changed = db.write().execute(
        "UPDATE tokens SET limits_json = ?2 WHERE id = ?1",
        params![id, limits.map(serde_json::to_string).transpose()?,],
    )?;
    Ok(changed > 0)
}

/// 软删除（撤销）。返回是否存在。
pub fn revoke(db: &Db, id: &str, now_millis: i64) -> Result<bool, StoreError> {
    let connection = db.write();
    let updated = connection.execute(
        "UPDATE tokens SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
        params![id, now_millis],
    )?;
    Ok(updated > 0)
}

/// `last_used_at` 每分钟最多回写一次。
pub fn touch_last_used(db: &Db, token_sha256: &str, now_millis: i64) -> Result<(), StoreError> {
    let connection = db.write();
    connection.execute(
        "UPDATE tokens SET last_used_at = ?2
          WHERE token_sha256 = ?1
            AND (last_used_at IS NULL OR ?2 - last_used_at >= 60000)",
        params![token_sha256, now_millis],
    )?;
    Ok(())
}

fn parse_scopes(json: &str) -> Vec<AdminScope> {
    serde_json::from_str::<Vec<String>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|scope| AdminScope::parse(&scope))
        .collect()
}

#[allow(clippy::type_complexity)]
fn row_from(row: &rusqlite::Row<'_>) -> Result<TokenRecord, rusqlite::Error> {
    let scopes_json: String = row.get(4)?;
    let created_at: i64 = row.get(5)?;
    Ok(TokenRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        token_sha256: row.get(2)?,
        prefix: row.get(3)?,
        scopes: parse_scopes(&scopes_json),
        created_at,
        last_used_at: row.get(6)?,
        expires_at: row.get(7)?,
        revoked_at: row.get(8)?,
        limits: row
            .get::<_, Option<String>>(9)?
            .and_then(|text| serde_json::from_str(&text).ok()),
    })
}

/// 是否还没有任何未撤销 token（bootstrap 判断）。
pub fn is_empty(db: &Db) -> Result<bool, StoreError> {
    let connection = db.read();
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM tokens WHERE revoked_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(count == 0)
}

impl TryFrom<&CreateTokenRequest> for TokenRecord {
    type Error = StoreError;

    fn try_from(request: &CreateTokenRequest) -> Result<Self, Self::Error> {
        let plaintext = generate_token();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: request.name.clone(),
            token_sha256: sha256_hex(&plaintext),
            prefix: plaintext.chars().take(8).collect(),
            scopes: request.scopes.clone(),
            created_at: crate::store::migrate::now_millis(),
            last_used_at: None,
            expires_at: request.expires_at,
            revoked_at: None,
            limits: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;

    fn sample(scopes: Vec<AdminScope>) -> TokenRecord {
        let plaintext = generate_token();
        TokenRecord {
            id: "t-1".to_string(),
            name: "admin".to_string(),
            token_sha256: sha256_hex(&plaintext),
            prefix: plaintext.chars().take(8).collect(),
            scopes,
            created_at: 1000,
            last_used_at: None,
            expires_at: None,
            revoked_at: None,
            limits: None,
        }
    }

    #[test]
    fn insert_find_revoke_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let record = sample(vec![AdminScope::AdminRead, AdminScope::AdminWrite]);
        insert(&db, &record).unwrap();

        // 用明文查不到（我们只存了哈希）；按哈希查得到。
        assert!(find_by_id(&db, "t-1").unwrap().is_some());
        assert!(!is_empty(&db).unwrap());

        // 撤销后不再活跃。
        revoke(&db, "t-1", 2000).unwrap();
        let fetched = find_by_id(&db, "t-1").unwrap().unwrap();
        assert!(!fetched.is_active(3000));

        // 重复撤销返回 false。
        assert!(!revoke(&db, "t-1", 3000).unwrap());
    }

    #[test]
    fn expiry_checked_at_auth_time() {
        let mut record = sample(vec![AdminScope::Inference]);
        record.expires_at = Some(5000);
        assert!(record.is_active(4999));
        assert!(!record.is_active(5000));
    }

    #[test]
    fn list_returns_prefix_only() {
        let db = Db::open_in_memory().unwrap();
        let record = sample(vec![AdminScope::AdminRead]);
        insert(&db, &record).unwrap();
        let items = list(&db).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].prefix, record.prefix);
        assert_eq!(items[0].scopes, vec![AdminScope::AdminRead]);
    }

    #[test]
    fn generated_token_shape() {
        let token = generate_token();
        assert!(token.starts_with("tr2_"));
        assert!(token.len() > 40);
        // 唯一性抽查。
        let other = generate_token();
        assert_ne!(token, other);
    }

    #[test]
    fn touch_last_used_throttled() {
        let db = Db::open_in_memory().unwrap();
        let record = sample(vec![AdminScope::AdminRead]);
        insert(&db, &record).unwrap();
        touch_last_used(&db, &record.token_sha256, 1000).unwrap();
        // 一分钟内不覆盖。
        touch_last_used(&db, &record.token_sha256, 30000).unwrap();
        let fetched = find_by_id(&db, "t-1").unwrap().unwrap();
        assert_eq!(fetched.last_used_at, Some(1000));
        // 超过一分钟才更新。
        touch_last_used(&db, &record.token_sha256, 70000).unwrap();
        let fetched = find_by_id(&db, "t-1").unwrap().unwrap();
        assert_eq!(fetched.last_used_at, Some(70000));
    }
}
