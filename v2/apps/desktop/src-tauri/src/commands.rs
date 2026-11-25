//! Tauri 命令层：supervisor / admin 代理 / host 命令。
//!
//! 独立成模块以便 `generate_handler![commands::...]` 路径解析（v1 同款模式）。

use std::sync::atomic::Ordering;

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
        // true = 连接外部服务（不托管进程）；前端只在 external === false 时
        // 显示本地启停动作，未知（旧宿主缺字段）一律不显示。
        "external": state.supervisor.is_external(),
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
    // 外部模式：远端进程不归本壳托管，明确报错（前端据此隐藏重启动作）。
    if state.supervisor.is_external() {
        return Err(
            "restart unavailable in external mode: service is not managed by this shell"
                .to_string(),
        );
    }
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

// ---------------------------------------------------------------------------
// 退出前询问（设置页「退出前询问」）
// ---------------------------------------------------------------------------

/// 开关退出守卫：开启时窗口关闭会被拦下，交回前端确认。
#[tauri::command]
pub fn app_set_exit_guard(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.exit_guard.store(enabled, Ordering::SeqCst);
    if !enabled {
        state.exit_confirmed.store(false, Ordering::SeqCst);
    }
    Ok(())
}

/// 用户确认退出：置位确认标记后关闭窗口（这次关闭不再被拦截）。
#[tauri::command]
pub fn app_force_close(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.exit_confirmed.store(true, Ordering::SeqCst);
    let _ = app;
    window.close().map_err(|error| error.to_string())
}

/// 用户取消退出：清掉确认标记，下次关闭仍会询问。
#[tauri::command]
pub fn app_cancel_exit(state: State<'_, AppState>) -> Result<(), String> {
    state.exit_confirmed.store(false, Ordering::SeqCst);
    Ok(())
}

/// 开关「关闭到托盘」：开启时窗口关闭 = 隐藏到托盘，服务继续跑。
#[tauri::command]
pub fn app_set_close_to_tray(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.close_to_tray.store(enabled, Ordering::SeqCst);
    Ok(())
}

/// 真退出（托盘「退出」/确认对话框）：绕过关闭到托盘，置退出标记后关窗。
///
/// 有退出守卫时这次 close 仍会交给前端确认（`app://close-requested`）；
/// 确认对话框里的「退出」再走 `app_force_close`。
#[tauri::command]
pub fn app_quit(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.quit_requested.store(true, Ordering::SeqCst);
    let _ = app;
    window.close().map_err(|error| error.to_string())
}

/// 监听地址 → base URL（reqwest 要绝对 URL，必须带协议前缀）。
///
/// 外部模式（`TR_EXTERNAL_BASE_URL`）给的已经是完整 URL，直接用，
/// 否则会拼成 `http://http://…`。
fn base_url(listen: Option<&str>) -> Option<String> {
    crate::base_url_of(listen)
}

async fn finish_ready(app: &AppHandle, state: &State<'_, AppState>, outcome: &SpawnOutcome) {
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
pub fn host_open_path(state: State<'_, AppState>, path: Option<String>) -> Result<(), String> {
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
pub fn host_ui_state_save(
    state: State<'_, AppState>,
    ui_state: host::UiState,
) -> Result<(), String> {
    host::ui_state_save(&state.data_dir, &ui_state)
}

/// 本机可用于局域网访问的 IPv4 地址（不含回环）。
///
/// 只读探测，不改任何网络配置：先用 UDP connect 拿到出站网卡地址
/// （connect 不发包），失败再回退解析 `ipconfig`。
#[tauri::command]
pub fn host_lan_addresses() -> Result<Vec<String>, String> {
    Ok(lan_ipv4())
}

fn lan_ipv4() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        // TEST-NET-1（RFC 5737）：connect 不会真的发包，只是让系统选路。
        if socket.connect("192.0.2.1:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                let ip = addr.ip();
                if !ip.is_loopback() && !ip.is_unspecified() {
                    out.push(ip.to_string());
                }
            }
        }
    }
    if out.is_empty() {
        if let Ok(output) = std::process::Command::new("ipconfig").output() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let Some((_, value)) = line.split_once(':') else {
                    continue;
                };
                let candidate = value.trim();
                let parts: Vec<&str> = candidate.split('.').collect();
                if parts.len() != 4 {
                    continue;
                }
                if !parts
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
                {
                    continue;
                }
                if candidate.starts_with("127.") {
                    continue;
                }
                if !out.iter().any(|item| item == candidate) {
                    out.push(candidate.to_string());
                }
            }
        }
    }
    out
}

fn open_in_file_manager(target: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(
                target
                    .canonicalize()
                    .unwrap_or_else(|_| target.to_path_buf()),
            )
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
