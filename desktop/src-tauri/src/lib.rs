use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent,
};

use token_router::embedded;

mod agent_setup;
mod devtools_gate;
mod feedback;
mod flowy_test_server;
mod herdsman;
pub mod ota;
mod status_pipe;
mod wechat_interceptor;

#[derive(Clone, serde::Serialize)]
struct GatewayStatus {
    running: bool,
    url: Option<String>,
    version: String,
}

#[tauri::command]
fn gateway_start() -> Result<String, String> {
    let result = if embedded::is_running() {
        embedded::gateway_url().ok_or_else(|| "gateway already running".to_string())
    } else {
        embedded::start(None).map_err(|e| e.to_string())
    };
    if result.is_ok() {
        status_pipe::sync_gateway_state();
    }
    result
}

#[tauri::command]
fn gateway_stop() -> Result<(), String> {
    if !embedded::is_running() {
        status_pipe::sync_gateway_state();
        return Ok(());
    }
    let result = embedded::stop().map_err(|e| e.to_string());
    status_pipe::sync_gateway_state();
    result
}

#[tauri::command]
fn gateway_restart() -> Result<String, String> {
    if embedded::is_running() {
        embedded::stop().map_err(|e| e.to_string())?;
    }
    let result = embedded::start(None).map_err(|e| e.to_string());
    status_pipe::sync_gateway_state();
    result
}

#[tauri::command]
fn gateway_is_running() -> bool {
    embedded::is_running()
}

#[tauri::command]
fn gateway_url() -> Option<String> {
    embedded::gateway_url()
}

#[tauri::command]
fn gateway_status() -> GatewayStatus {
    GatewayStatus {
        running: embedded::is_running(),
        url: embedded::gateway_url(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
fn gateway_read_logs(
    offset: Option<u64>,
    before_offset: Option<u64>,
) -> Result<token_router::gateway::logging::LogsTail, String> {
    let config = token_router::gateway::AppConfig::load().map_err(|e| e.to_string())?;
    let path = config.data_dir.join("logs").join("gateway.log");
    if let Some(before) = before_offset {
        token_router::gateway::logging::read_log_before(&path, before, None)
            .map_err(|e| e.to_string())
    } else {
        token_router::gateway::logging::read_log_tail(&path, offset, None).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn gateway_read_routing_logs(
    after_id: Option<i64>,
    before_id: Option<i64>,
    limit: Option<u32>,
) -> Result<token_router::gateway::routing_log::RoutingLogsResponse, String> {
    let config = token_router::gateway::AppConfig::load().map_err(|e| e.to_string())?;
    let store = token_router::gateway::routing_log::RoutingLogStore::open(&config.data_dir)
        .map_err(|e| e.to_string())?;
    store
        .query(token_router::gateway::routing_log::RoutingLogsQuery {
            after_id,
            before_id,
            limit,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn gateway_open_logs_dir() -> Result<(), String> {
    let config = token_router::gateway::AppConfig::load().map_err(|e| e.to_string())?;
    let logs_dir = config.data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).map_err(|e| e.to_string())?;
    tauri_plugin_opener::open_path(logs_dir, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Hide to tray without blocking the WebView2 message loop (Windows deadlock otherwise).
#[tauri::command]
async fn hide_main_window(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn(async move {
        if let Some(window) = app.get_webview_window("main") {
            if let Err(err) = window.hide() {
                eprintln!("hide_main_window: {err}");
            }
        }
    });
    Ok(())
}

fn defer_show_main_window(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
}

fn defer_quit(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        app.exit(0);
    });
}

fn wechat_callback_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("wechat-callback")
        .on_navigation(|webview, url| {
            let url_str = url.as_str();
            if url_str.contains("/auth/third/callback") && !url_str.contains("channel=") {
                emit_wechat_callback(webview, url_str);
                return false;
            }
            if url_str.contains("localhost.weixin.qq.com") {
                return false;
            }
            true
        })
        .on_page_load(|webview, payload| {
            if payload.event() != tauri::webview::PageLoadEvent::Finished {
                return;
            }
            let url_str = payload.url().as_str();
            if url_str.contains("/auth/third/callback") && !url_str.contains("channel=") {
                emit_wechat_callback(webview, url_str);
            }
        })
        .build()
}

fn emit_wechat_callback<R: tauri::Runtime>(webview: &tauri::Webview<R>, url: &str) {
    let _ = webview.emit("wechat-login-callback", url);
    let app = webview.app_handle();
    let _ = app.emit_to(webview.label(), "wechat-login-callback", url);
}

fn stop_embedded_with_timeout(timeout: Duration) {
    if !embedded::is_running() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(embedded::stop());
    });
    if rx.recv_timeout(timeout).is_err() {
        eprintln!(
            "embedded gateway stop timed out after {}s",
            timeout.as_secs()
        );
    }
}

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

fn shutdown_background_services() {
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    ota::stop_background_checks();
    herdsman::stop_herdsman_service();
    status_pipe::stop();
    stop_embedded_with_timeout(Duration::from_secs(5));
}

fn load_app_icon(app: &tauri::App) -> Image<'static> {
    app.default_window_icon()
        .map(|img| img.clone().to_owned())
        .unwrap_or_else(|| {
            Image::from_bytes(include_bytes!("../icons/32x32.png"))
                .expect("load app icon")
        })
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "Show / 显示", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit / 退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let icon = load_app_icon(app);

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Token Router")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                defer_show_main_window(app);
            }
            "quit" => {
                defer_quit(app);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                defer_show_main_window(&app);
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            defer_show_main_window(&app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(wechat_callback_plugin())
        .invoke_handler(tauri::generate_handler![
            agent_setup::configure_openclaw_agent,
            agent_setup::configure_hermes_agent,
            agent_setup::configure_hermes_flash_agent,
            agent_setup::configure_claude_code_agent,
            agent_setup::configure_codex_agent,
            agent_setup::read_inbound_auth_key_cmd,
            agent_setup::check_agent_initialized,
            agent_setup::check_agent_deployed,
            gateway_start,
            gateway_stop,
            gateway_restart,
            gateway_is_running,
            gateway_url,
            gateway_status,
            gateway_read_logs,
            gateway_read_routing_logs,
            gateway_open_logs_dir,
            show_main_window,
            hide_main_window,
            herdsman::herdsman_open_or_install,
            herdsman::herdsman_get_status,
            herdsman::herdsman_refresh_status,
            herdsman::herdsman_start,
            feedback::feedback_app_version,
            feedback::feedback_submit,
            ota::service::ota_app_version,
            ota::service::ota_check_now,
            ota::service::ota_download_update,
            ota::service::ota_do_update,
            ota::service::ota_get_post_restart_notice,
        ])
        .manage(ota::service::manage_state())
        .setup(|app| {
            setup_tray(app)?;
            status_pipe::start();
            if embedded::is_running() {
                status_pipe::sync_gateway_state();
            }
            herdsman::start_herdsman_service(app.handle().clone());
            ota::start_background_checks(app.handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_icon(load_app_icon(app));
                let app_handle = app.handle().clone();
                let label = window.label().to_string();
                let dev_access = flowy_test_server::flowy_test_server_enabled();
                let _ = window.with_webview(move |platform| {
                    devtools_gate::apply_dev_access(&platform, dev_access);
                    wechat_interceptor::register_wechat_iframe_interceptor(
                        platform,
                        app_handle,
                        label,
                    );
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide to tray instead of quitting (quit via tray menu).
                // Do not call hide() synchronously here — WebView2 deadlocks on Windows.
                api.prevent_close();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                });
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let RunEvent::Exit = event {
                // Do not block the Windows message loop during teardown.
                std::thread::spawn(shutdown_background_services);
            }
        });
}
