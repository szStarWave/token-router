//! Tauri 命令层：supervisor / admin 代理 / host 命令。
//!
//! 独立成模块以便 `generate_handler![commands::...]` 路径解析（v1 同款模式）。

use tauri::{AppHandle, Emitter, State};

use crate::host;
use crate::supervisor::{SpawnOutcome, SupervisionState};
use crate::AppState;

// ---------------------------------------------------------------------------
// supervisor 命令
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn supervisor_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let supervisor_state = state.supervisor.state().await;
    let listen = state.supervisor.listen_addr().await;
    Ok(serde_json::json!({
        "state": supervisor_state.as_str(),
        "listen": listen,
        "data_dir": state.data_dir.display().to_string(),
    }))
}

#[tauri::command]
pub async fn supervisor_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let outcome = state.supervisor.start().await;
    let ready = outcome.state == SupervisionState::Ready;
    if ready {
        finish_ready(&app, &state, &outcome).await;
    }
    Ok(serde_json::json!({
        "state": outcome.state.as_str(),
        "listen": outcome.listen,
        "message": outcome.message,
    }))
}

#[tauri::command]
pub async fn supervisor_restart(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let outcome = state.supervisor.restart_with_backoff().await;
    if outcome.state == SupervisionState::Ready {
        *state.base_url.lock().await = base_url(outcome.listen.as_deref());

        let token_path = state.data_dir.join("admin.token");
        if let Ok(token) = tokio::fs::read_to_string(&token_path).await {
            *state.admin_token.lock().await = Some(token.trim().to_string());
        }
    }
    Ok(serde_json::json!({
        "state": outcome.state.as_str(),
        "listen": outcome.listen,
        "message": outcome.message,
    }))
}

#[tauri::command]
pub async fn supervisor_stop(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let message = state.supervisor.stop().await;
    *state.base_url.lock().await = None;
    Ok(serde_json::json!({"state": "stopped", "message": message}))
}

/// 监听地址 → base URL（reqwest 要绝对 URL，必须带协议前缀）。
fn base_url(listen: Option<&str>) -> Option<String> {
    listen.map(|listen| format!("http://{listen}"))
}

async fn finish_ready(
    app: &AppHandle,
    state: &State<'_, AppState>,
    outcome: &SpawnOutcome,
) {
    *state.base_url.lock().await = base_url(outcome.listen.as_deref());
    let token_path = state.data_dir.join("admin.token");
    if let Ok(token) = tokio::fs::read_to_string(&token_path).await {
        *state.admin_token.lock().await = Some(token.trim().to_string());
    }
    let _ = app.emit("supervisor://state", serde_json::json!({"state": "ready"}));
}

// ---------------------------------------------------------------------------
// admin 代理（token 注入在 Rust 侧）
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn admin_request(
    state: State<'_, AppState>,
    request: crate::admin_proxy::AdminRequest,
) -> Result<crate::admin_proxy::AdminResponse, String> {
    let base = state.base_url.lock().await.clone();
    let Some(base) = base else {
        return Ok(crate::admin_proxy::AdminResponse {
            status: 503,
            body: None,
            error: Some(serde_json::json!({
                "error": {
                    "type": "server_unavailable",
                    "message": "service not running; start it first",
                }
            })),
        });
    };
    let token = state.admin_token.lock().await.clone().unwrap_or_default();
    Ok(crate::admin_proxy::forward(&base, &token, &request).await)
}

// ---------------------------------------------------------------------------
// host 命令
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn host_app_info(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "data_dir": state.data_dir.display().to_string(),
    }))
}

#[tauri::command]
pub fn host_open_path(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<(), String> {
    let target = match path {
        Some(relative) => state.data_dir.join(relative),
        None => state.data_dir.clone(),
    };
    host::open_path_allowed(&state.data_dir, &target)?;
    open_in_file_manager(&target)
}

#[tauri::command]
pub fn host_ui_state_load(state: State<'_, AppState>) -> Result<host::UiState, String> {
    Ok(host::ui_state_load(&state.data_dir))
}

#[tauri::command]
pub fn host_ui_state_save(state: State<'_, AppState>, ui_state: host::UiState) -> Result<(), String> {
    host::ui_state_save(&state.data_dir, &ui_state)
}

fn open_in_file_manager(target: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(target.canonicalize().unwrap_or_else(|_| target.to_path_buf()))
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(target)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(target)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
