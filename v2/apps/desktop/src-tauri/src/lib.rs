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
pub mod events_bridge;
pub mod host;
pub mod onboard;
pub mod supervisor;

use std::path::PathBuf;

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
    let mut candidates: Vec<PathBuf> = Vec::new();
    // 当前进程所在目录及其祖先（cargo test 的进程在 target/debug/deps/ 下，多退一级）。
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.to_path_buf()))
    {
        candidates.push(dir.join("token-router-v2.exe"));
        candidates.push(dir.join("token-router-v2"));
        // 沿祖先链找 debug/token-router-v2（deps/、build/.../out/ 深度不定）。
        let mut ancestors = dir.clone();
        for _ in 0..8 {
            if let Some(parent) = ancestors.parent() {
                ancestors = parent.to_path_buf();
                candidates.push(ancestors.join("debug").join("token-router-v2.exe"));
                candidates.push(ancestors.join("debug").join("token-router-v2"));
            }
        }
        // 显式 workspace 路径兜底（apps/desktop/src-tauri -> v2）。
        if let Some(v2) = dir.ancestors().nth(4) {
            candidates.push(v2.join("target").join("debug").join("token-router-v2.exe"));
            candidates.push(v2.join("target").join("debug").join("token-router-v2"));
        }
    }
    // 打包布局：sidecar 与主程序同目录（externalBin）。
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("sidecar").join("token-router-v2.exe"));
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

/// 监听地址 → base URL（reqwest 要绝对 URL）。
///
/// 外部模式（`TR_EXTERNAL_BASE_URL`）给的已经是完整 URL，直接用，
/// 否则会拼成 `http://http://…`。
pub(crate) fn base_url_of(listen: Option<&str>) -> Option<String> {
    listen.map(|listen| {
        if listen.starts_with("http://") || listen.starts_with("https://") {
            listen.to_string()
        } else {
            format!("http://{listen}")
        }
    })
}

/// 运行 tauri 应用（main.rs / mobile entry 共用）。
pub fn run() {
    let data_dir = default_data_dir();
    let _ = std::fs::create_dir_all(&data_dir);
    // 注意：manage() 按类型登记状态，命令侧取的是 State<'_, AppState>，
    // 所以这里必须登记 AppState 本身，包一层 Arc 会取不到（state not managed）。
    let state = AppState::new(data_dir);

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
        .manage(events_bridge::EventBridge::default())
        .setup(|app| {
            // 外部服务模式：启动即连上，省掉一次手动「启动服务」。
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async {
                let state = handle.state::<AppState>();
                if state.supervisor.external_base().is_none() {
                    return;
                }
                let outcome = state.supervisor.start().await;
                if outcome.state != supervisor::SupervisionState::Ready {
                    return;
                }
                *state.base_url.lock().await = base_url_of(outcome.listen.as_deref());
                let token_path = state.data_dir.join("admin.token");
                if let Ok(token) = tokio::fs::read_to_string(&token_path).await {
                    *state.admin_token.lock().await = Some(token.trim().to_string());
                }
            });
            Ok(())
        })
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
            onboard::onboard_detect,
            onboard::onboard_plan,
            onboard::onboard_apply,
            onboard::onboard_verify,
            onboard::onboard_wsl_detect,
            events_bridge::admin_events_subscribe,
            events_bridge::admin_events_unsubscribe,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use supervisor::{SupervisionState, SupervisorConfig};

    /// 测试用最小配置：一个指向本地占位端口的候选 + 一个虚拟模型。
    /// 上游不会在启动时被访问，因此无需真实上游或凭据。
    const TEST_CONFIG: &str = r#"
schema_version = 1
default_virtual_model = "auto"

[server]
listen = "127.0.0.1:17621"

[profiles.default]
mode = "cascade"

[[candidates]]
id = "test-local"
tier = "local"
strength = 10
provider = "openai"
base_url = "http://127.0.0.1:9/v1"
model = "test-model"
window = 8192
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["test-local"]
"#;

    /// 端到端拉起真实 sidecar。
    ///
    /// 需要先构建被测二进制，否则没有 sidecar 可拉：
    ///   cd v2 && cargo build -p token-router-v2
    /// 因此默认忽略，手动跑：`cargo test -- --ignored`。
    #[tokio::test]
    #[ignore = "需要先构建 v2 的 token-router-v2 二进制"]
    async fn supervisor_starts_real_sidecar() {
        let data_dir =
            std::env::temp_dir().join(format!("tr-supervisor-it-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&data_dir);
        std::fs::write(data_dir.join("config.toml"), TEST_CONFIG).expect("write config");

        let supervisor = Supervisor::new(SupervisorConfig {
            sidecar_path: sidecar_binary(),
            data_dir: data_dir.clone(),
            listen: "127.0.0.1:17621".to_string(),
            external: None,
        });
        let outcome = supervisor.start().await;
        if outcome.state == SupervisionState::Ready {
            let _ = supervisor.stop().await;
        }
        let _ = std::fs::remove_dir_all(&data_dir);

        assert_eq!(
            outcome.state,
            SupervisionState::Ready,
            "message: {}",
            outcome.message
        );
    }

    /// sidecar 路径解析：dev 布局下应能找到 v2 workspace 的 debug 二进制。
    ///
    /// 依赖被测二进制已构建（同上），因此默认忽略。
    #[test]
    #[ignore = "需要先构建 v2 的 token-router-v2 二进制"]
    fn sidecar_binary_resolves_in_dev() {
        let path = sidecar_binary();
        assert!(path.is_file(), "sidecar not found: {}", path.display());
    }
}
