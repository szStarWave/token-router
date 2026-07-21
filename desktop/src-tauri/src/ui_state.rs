use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct UiState {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    theme: String,
    locale: String,
    #[serde(rename = "hasSeenOnboarding")]
    has_seen_onboarding: bool,
    #[serde(rename = "edgeUserConfigured")]
    edge_user_configured: bool,
    #[serde(rename = "edgeManualEntries")]
    edge_manual_entries: Vec<serde_json::Value>,
    #[serde(rename = "cloudUserConfigured")]
    cloud_user_configured: bool,
    #[serde(rename = "cloudManualEntries")]
    cloud_manual_entries: Vec<serde_json::Value>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: "system".to_string(),
            locale: "zh".to_string(),
            has_seen_onboarding: false,
            edge_user_configured: false,
            edge_manual_entries: vec![],
            cloud_user_configured: false,
            cloud_manual_entries: vec![],
        }
    }
}

fn ui_state_path() -> Result<PathBuf, String> {
    // Prefer app_dir() so UI prefs work before the embedded gateway starts.
    let dir = token_router::config::app_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("ui-state.json"))
}

#[tauri::command]
pub fn ui_state_load() -> Result<UiState, String> {
    let path = ui_state_path()?;
    if !path.exists() {
        return Ok(UiState::default());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).or(Ok(UiState::default()))
}

#[tauri::command]
pub fn ui_state_save(state: UiState) -> Result<(), String> {
    let path = ui_state_path()?;
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    let tmp = dir.join("ui-state.json.tmp");
    let json = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, &json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;

    Ok(())
}
