use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::file::{ConfigFile, GatewayApiKeyEntry, GatewaySection};
use super::setup::{mask_gateway_api_key, normalize_gateway_api_key};

pub const DEFAULT_AUTH_KEY_NAME: &str = "Default";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayAuthKeyView {
    pub id: String,
    pub name: String,
    pub key_preview: String,
    pub created_at: i64,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateGatewayAuthKeyRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateGatewayAuthKeyResponse {
    pub key: GatewayAuthKeyView,
    #[serde(rename = "full_key")]
    pub full_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateGatewayAuthKeyRequest {
    pub name: String,
}

pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn generate_gateway_api_key_value() -> Result<String, String> {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    normalize_gateway_api_key(&format!("token-{suffix}"))
}

pub fn default_gateway_auth_key(gateway: &GatewaySection) -> Option<&GatewayApiKeyEntry> {
    gateway.api_keys.iter().find(|entry| entry.is_default)
}

pub fn default_gateway_auth_key_value(gateway: &GatewaySection) -> Option<String> {
    default_gateway_auth_key(gateway).map(|entry| entry.key.clone())
}

pub fn ensure_default_gateway_auth_key(file: &mut ConfigFile) -> bool {
    if file.gateway.api_keys.iter().any(|entry| entry.is_default) {
        return false;
    }

    if let Some(idx) = file
        .gateway
        .api_keys
        .iter()
        .position(|entry| entry.name == DEFAULT_AUTH_KEY_NAME)
    {
        file.gateway.api_keys[idx].is_default = true;
        let entry = file.gateway.api_keys.remove(idx);
        file.gateway.api_keys.insert(0, entry);
        return true;
    }

    let full_key = match generate_gateway_api_key_value() {
        Ok(key) => key,
        Err(_) => return false,
    };
    file.gateway.api_keys.insert(
        0,
        GatewayApiKeyEntry {
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: DEFAULT_AUTH_KEY_NAME.to_string(),
            key: full_key,
            created_at: now_unix_secs(),
            is_default: true,
        },
    );
    true
}

pub fn collect_inbound_api_keys(gateway: &GatewaySection) -> Vec<String> {
    let mut keys: Vec<String> = gateway
        .api_keys
        .iter()
        .map(|entry| entry.key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect();
    if let Some(legacy) = gateway.api_key.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !keys.iter().any(|key| key == legacy) {
            keys.push(legacy.to_string());
        }
    }
    keys
}

#[derive(Debug, Clone)]
pub struct ResolvedAuthKey {
    pub id: String,
    pub name: String,
    pub key_preview: String,
}

pub fn resolve_auth_key_by_value(gateway: &GatewaySection, key_value: &str) -> Option<ResolvedAuthKey> {
    for entry in &gateway.api_keys {
        if entry.key == key_value {
            return Some(ResolvedAuthKey {
                id: entry.id.clone(),
                name: entry.name.clone(),
                key_preview: mask_gateway_api_key(&entry.key)
                    .unwrap_or_else(|| "token-***".to_string()),
            });
        }
    }
    None
}

pub fn build_auth_key_by_value(gateway: &GatewaySection) -> std::collections::HashMap<String, ResolvedAuthKey> {
    let mut map = std::collections::HashMap::new();
    for entry in &gateway.api_keys {
        let key = entry.key.trim().to_string();
        if key.is_empty() {
            continue;
        }
        map.insert(
            key,
            ResolvedAuthKey {
                id: entry.id.clone(),
                name: entry.name.clone(),
                key_preview: mask_gateway_api_key(&entry.key)
                    .unwrap_or_else(|| "token-***".to_string()),
            },
        );
    }
    map
}

pub fn migrate_legacy_gateway_api_key(file: &mut ConfigFile) {
    if !file.gateway.api_keys.is_empty() {
        return;
    }
    let Some(legacy) = file
        .gateway
        .api_key
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    file.gateway.api_keys.push(GatewayApiKeyEntry {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: DEFAULT_AUTH_KEY_NAME.to_string(),
        key: legacy.to_string(),
        created_at: now_unix_secs(),
        is_default: false,
    });
}

fn entry_to_view(entry: &GatewayApiKeyEntry) -> GatewayAuthKeyView {
    GatewayAuthKeyView {
        id: entry.id.clone(),
        name: entry.name.clone(),
        key_preview: mask_gateway_api_key(&entry.key).unwrap_or_else(|| "token-***".to_string()),
        created_at: entry.created_at,
        is_default: entry.is_default,
    }
}

fn normalize_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name cannot be empty".into());
    }
    Ok(trimmed.to_string())
}

pub fn list_gateway_auth_keys(file: &ConfigFile) -> Vec<GatewayAuthKeyView> {
    file.gateway
        .api_keys
        .iter()
        .map(entry_to_view)
        .collect()
}

pub fn create_gateway_auth_key(
    file: &mut ConfigFile,
    name: &str,
) -> Result<(GatewayAuthKeyView, String), String> {
    let name = normalize_name(name)?;
    let full_key = generate_gateway_api_key_value()?;
    let entry = GatewayApiKeyEntry {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name,
        key: full_key.clone(),
        created_at: now_unix_secs(),
        is_default: false,
    };
    let view = entry_to_view(&entry);
    file.gateway.api_keys.push(entry);
    Ok((view, full_key))
}

pub fn update_gateway_auth_key_name(
    file: &mut ConfigFile,
    id: &str,
    name: &str,
) -> Result<GatewayAuthKeyView, String> {
    let name = normalize_name(name)?;
    let entry = file
        .gateway
        .api_keys
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "auth key not found".to_string())?;
    if entry.is_default {
        return Err("default auth key cannot be modified".into());
    }
    entry.name = name;
    Ok(entry_to_view(entry))
}

pub fn delete_gateway_auth_key(file: &mut ConfigFile, id: &str) -> Result<(), String> {
    let target = file
        .gateway
        .api_keys
        .iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "auth key not found".to_string())?;
    if target.is_default {
        return Err("default auth key cannot be deleted".into());
    }
    file.gateway.api_keys.retain(|entry| entry.id != id);
    if file.gateway.api_keys.is_empty() {
        file.gateway.api_key = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_legacy_api_key_into_list() {
        let mut file = ConfigFile::default();
        file.gateway.api_key = Some("token-abcdefghijklmnopqrstuvwxyz012345".into());
        migrate_legacy_gateway_api_key(&mut file);
        assert_eq!(file.gateway.api_keys.len(), 1);
        assert_eq!(file.gateway.api_keys[0].key, "token-abcdefghijklmnopqrstuvwxyz012345");
    }

    #[test]
    fn collect_inbound_keys_includes_legacy_and_list() {
        let mut file = ConfigFile::default();
        file.gateway.api_key = Some("token-legacylegacylegacylegacylegacy01".into());
        file.gateway.api_keys.push(GatewayApiKeyEntry {
            id: "a".into(),
            name: "A".into(),
            key: "token-newnewnewnewnewnewnewnewnew01".into(),
            created_at: 1,
            is_default: false,
        });
        let keys = collect_inbound_api_keys(&file.gateway);
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn ensure_default_creates_key_when_missing() {
        let mut file = ConfigFile::default();
        assert!(ensure_default_gateway_auth_key(&mut file));
        assert_eq!(file.gateway.api_keys.len(), 1);
        assert!(file.gateway.api_keys[0].is_default);
        assert_eq!(file.gateway.api_keys[0].name, DEFAULT_AUTH_KEY_NAME);
        assert!(file.gateway.api_keys[0].key.starts_with("token-"));
    }

    #[test]
    fn ensure_default_adopts_legacy_default_name() {
        let mut file = ConfigFile::default();
        file.gateway.api_keys.push(GatewayApiKeyEntry {
            id: "legacy".into(),
            name: DEFAULT_AUTH_KEY_NAME.into(),
            key: "token-abcdefghijklmnopqrstuvwxyz012345".into(),
            created_at: 1,
            is_default: false,
        });
        assert!(ensure_default_gateway_auth_key(&mut file));
        assert_eq!(file.gateway.api_keys.len(), 1);
        assert!(file.gateway.api_keys[0].is_default);
        assert_eq!(
            file.gateway.api_keys[0].key,
            "token-abcdefghijklmnopqrstuvwxyz012345"
        );
    }

    #[test]
    fn ensure_default_noop_when_already_present() {
        let mut file = ConfigFile::default();
        file.gateway.api_keys.push(GatewayApiKeyEntry {
            id: "d".into(),
            name: DEFAULT_AUTH_KEY_NAME.into(),
            key: "token-abcdefghijklmnopqrstuvwxyz012345".into(),
            created_at: 1,
            is_default: true,
        });
        assert!(!ensure_default_gateway_auth_key(&mut file));
        assert_eq!(file.gateway.api_keys.len(), 1);
    }

    #[test]
    fn default_gateway_auth_key_value_returns_key() {
        let mut file = ConfigFile::default();
        ensure_default_gateway_auth_key(&mut file);
        let value = default_gateway_auth_key_value(&file.gateway).unwrap();
        assert_eq!(value, file.gateway.api_keys[0].key);
    }

    #[test]
    fn update_default_key_rejected() {
        let mut file = ConfigFile::default();
        ensure_default_gateway_auth_key(&mut file);
        let id = file.gateway.api_keys[0].id.clone();
        let err = update_gateway_auth_key_name(&mut file, &id, "Renamed").unwrap_err();
        assert!(err.contains("cannot be modified"));
    }

    #[test]
    fn delete_default_key_rejected() {
        let mut file = ConfigFile::default();
        ensure_default_gateway_auth_key(&mut file);
        let id = file.gateway.api_keys[0].id.clone();
        let err = delete_gateway_auth_key(&mut file, &id).unwrap_err();
        assert!(err.contains("cannot be deleted"));
    }
}
