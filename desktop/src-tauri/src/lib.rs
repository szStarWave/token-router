use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent,
};

use token_router::embedded;

mod wechat_interceptor;
mod devtools_gate;

fn flowy_test_server_enabled() -> bool {
    match option_env!("FLOWY_TEST_SERVER_ENABLED") {
        Some("true") => true,
        _ => false,
    }
}

#[derive(Clone, serde::Serialize)]
struct GatewayStatus {
    running: bool,
    url: Option<String>,
    version: String,
}

#[tauri::command]
fn gateway_start() -> Result<String, String> {
    if embedded::is_running() {
        return embedded::gateway_url().ok_or_else(|| "gateway already running".to_string());
    }
    embedded::start(None).map_err(|e| e.to_string())
}

#[tauri::command]
fn gateway_stop() -> Result<(), String> {
    if !embedded::is_running() {
        return Ok(());
    }
    embedded::stop().map_err(|e| e.to_string())
}

#[tauri::command]
fn gateway_restart() -> Result<String, String> {
    if embedded::is_running() {
        embedded::stop().map_err(|e| e.to_string())?;
    }
    embedded::start(None).map_err(|e| e.to_string())
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
fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
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
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                if embedded::is_running() {
                    let _ = embedded::stop();
                }
                app.exit(0);
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
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(wechat_callback_plugin())
        .invoke_handler(tauri::generate_handler![
            gateway_start,
            gateway_stop,
            gateway_restart,
            gateway_is_running,
            gateway_url,
            gateway_status,
            show_main_window,
        ])
        .setup(|app| {
            setup_tray(app)?;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_icon(load_app_icon(app));
                let app_handle = app.handle().clone();
                let label = window.label().to_string();
                let dev_access = flowy_test_server_enabled();
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
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let RunEvent::Exit = event {
                if embedded::is_running() {
                    let _ = embedded::stop();
                }
            }
        });
}
