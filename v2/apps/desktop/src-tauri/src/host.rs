//! host 命令（技术方案 §6.4）：ui-state / 打开目录 / 应用信息。
//!
//! 白名单：`host_open_path` 只允许打开数据目录与其子目录（logs 等）。

use serde::{Deserialize, Serialize};

/// 应用信息。
#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub data_dir: String,
}

/// ui-state（主题/语言/窗口/tour）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub sidebar_collapsed: Option<bool>,
    #[serde(default)]
    pub tour_done: Option<bool>,
    #[serde(default)]
    pub window: Option<WindowGeometry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// 加载 ui-state（文件不存在返回默认）。
pub fn ui_state_load(data_dir: &std::path::Path) -> UiState {
    let path = data_dir.join("ui-state.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 保存 ui-state（原子写：临时文件 + rename）。
pub fn ui_state_save(data_dir: &std::path::Path, state: &UiState) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let path = data_dir.join("ui-state.json");
    let temp = data_dir.join("ui-state.json.tmp");
    let text = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    std::fs::write(&temp, text).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// 打开目录（白名单：数据目录自身或其子目录）。
///
/// 不要求目标存在：先尝试 canonicalize（解析 symlink/大小写），
/// 失败则退回词法路径（新目录场景）。Windows canonicalize 带 `\\?\` 前缀，
/// 统一在字符串层面比较以避免前缀差异。
pub fn open_path_allowed(
    data_dir: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    let normalize = |path: &std::path::Path| -> String {
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches(['/', '\\'])
            .to_string()
    };
    let data = normalize(data_dir);
    let candidate = normalize(target);
    if candidate == data
        || candidate.starts_with(&format!("{data}\\"))
        || candidate.starts_with(&format!("{data}/"))
    {
        Ok(())
    } else {
        Err(format!(
            "path outside data dir rejected: {}",
            target.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_state_roundtrip() {
        let dir = std::env::temp_dir().join("tr-desktop-test-ui");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ui_state_save(
            &dir,
            &UiState {
                theme: Some("dark".into()),
                tour_done: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let loaded = ui_state_load(&dir);
        assert_eq!(loaded.theme.as_deref(), Some("dark"));
        assert_eq!(loaded.tour_done, Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_path_whitelist() {
        let dir = std::env::temp_dir().join("tr-desktop-test-open");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(open_path_allowed(&dir, &dir.join("logs")).is_ok());
        assert!(open_path_allowed(&dir, dir.parent().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
