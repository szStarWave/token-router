use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::file::{ConfigFile, GatewayApiKeyEntry, GatewaySection};
use super::setup::{mask_gateway_api_key, normalize_gateway_api_key};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayAuthKeyView {
    pub id: String,
    pub name: String,
    pub key_preview: String,
    pub created_at: i64,
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
        name: "Default".to_string(),
        key: legacy.to_string(),
        created_at: now_unix_secs(),
    });
}

fn entry_to_view(entry: &GatewayApiKeyEntry) -> GatewayAuthKeyView {
    GatewayAuthKeyView {
        id: entry.id.clone(),
        name: entry.name.clone(),
        key_preview: mask_gateway_api_key(&entry.key).unwrap_or_else(|| "token-***".to_string()),
        created_at: entry.created_at,
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
    entry.name = name;
    Ok(entry_to_view(entry))
}

pub fn delete_gateway_auth_key(file: &mut ConfigFile, id: &str) -> Result<(), String> {
    let before = file.gateway.api_keys.len();
    file.gateway.api_keys.retain(|entry| entry.id != id);
    if file.gateway.api_keys.len() == before {
        return Err("auth key not found".into());
    }
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
        });
        let keys = collect_inbound_api_keys(&file.gateway);
        assert_eq!(keys.len(), 2);
    }
}
