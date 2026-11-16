//! Token Router v2 桌面壳（Tauri 2）。
//!
//! 责任边界（技术方案 §3.1）：
//! - supervisor：拉起/守护 `token-router-v2` sidecar；
//! - admin 代理：token 只在 Rust 侧内存，不进 WebView；
//! - host 命令：ui-state / 打开目录 / 应用信息；
//! - UI 只通过 `admin_request` / `supervisor_*` 访问服务端，不读文件/DB。

#![forbid(unsafe_code)]

pub mod admin_proxy;
pub mod commands;
pub mod host;
pub mod supervisor;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use supervisor::Supervisor;
use supervisor::SupervisorConfig;

/// 壳的运行时状态。
pub struct AppState {
    pub supervisor: Supervisor,
    /// admin token（Rust 侧内存；不返回给 JS）。
    pub admin_token: Mutex<Option<String>>,
    /// 服务端 base URL。
    pub base_url: Mutex<Option<String>>,
    /// 数据目录。
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Self {
        let sidecar_path = sidecar_binary();
        let supervisor = Supervisor::new(SupervisorConfig {
            sidecar_path,
            listen: "127.0.0.1:17621".to_string(),
            external: std::env::var("TR_EXTERNAL_BASE_URL").ok(),
            data_dir: data_dir.clone(),
        });
        Self {
            supervisor,
            admin_token: Mutex::new(None),
            base_url: Mutex::new(None),
            data_dir,
        }
    }
}

/// sidecar 路径：与可执行文件同目录的 `token-router-v2(.exe)`；开发期回退到 target/debug。
fn sidecar_binary() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.to_path_buf()))
    {
        candidates.push(dir.join("token-router-v2.exe"));
        candidates.push(dir.join("token-router-v2"));
        // cargo dev：workspace target/debug（src-tauri/target -> v2/target）
        if let Some(v2) = dir.parent().and_then(|p| p.parent()) {
            candidates.push(v2.join("target").join("debug").join("token-router-v2.exe"));
            candidates.push(v2.join("target").join("debug").join("token-router-v2"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("token-router-v2"))
}

/// 默认数据目录：`~/.token-router-v2`。
pub fn default_data_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    PathBuf::from(home).join(".token-router-v2")
}

/// 运行 tauri 应用（main.rs / mobile entry 共用）。
pub fn run() {
    let data_dir = default_data_dir();
    let _ = std::fs::create_dir_all(&data_dir);
    let state = Arc::new(AppState::new(data_dir));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 第二实例：唤起主窗口。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::supervisor_status,
            commands::supervisor_start,
            commands::supervisor_restart,
            commands::supervisor_stop,
            commands::admin_request,
            commands::host_app_info,
            commands::host_open_path,
            commands::host_ui_state_load,
            commands::host_ui_state_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
