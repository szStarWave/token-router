use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const POST_OTA_RESTART_NOTICE_FILE: &str = "post_ota_restart_notice.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostOtaRestartNotice {
    #[serde(default)]
    pub show: bool,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub release_notes: std::collections::HashMap<String, Vec<String>>,
}

pub fn notice_path(data_dir: &Path) -> PathBuf {
    data_dir.join(POST_OTA_RESTART_NOTICE_FILE)
}

pub fn write_post_ota_restart_notice(data_dir: &Path, notice: &PostOtaRestartNotice) -> Result<(), String> {
    let path = notice_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(notice).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn read_and_consume_post_ota_restart_notice(data_dir: &Path) -> Result<PostOtaRestartNotice, String> {
    let path = notice_path(data_dir);
    if !path.is_file() {
        return Ok(PostOtaRestartNotice {
            show: false,
            version: String::new(),
            release_notes: std::collections::HashMap::new(),
        });
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

pub fn data_dir() -> Result<PathBuf, String> {
    let config = token_router::gateway::AppConfig::load().map_err(|e| e.to_string())?;
    Ok(config.data_dir)
}
