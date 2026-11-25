//! 系统托盘。
//!
//! 常驻托盘：应用启动即创建；左键点击恢复/聚焦主窗口。
//! 菜单三项：显示主窗口 / 服务状态（后台定期刷新）/ 退出。
//! 「退出」置真退出标记后走窗口 close：退出守卫（`exit_guard`）在 Rust 侧
//! 拦截，交回前端确认；确认后再关窗，`RunEvent::Exit` 统一停 sidecar。

use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager,
};

/// 服务状态菜单项文本：状态 + 监听地址 → 展示词。
/// 纯函数，测试覆盖；调用方负责取状态（菜单构建用非阻塞近似值，
/// 后台刷新任务用完整异步读取）。
fn status_text(state: &str, listen: Option<&str>) -> String {
    match (state, listen) {
        ("ready", Some(listen)) => format!("服务运行中 · {listen}"),
        ("ready", None) => "服务运行中".to_string(),
        (state, _) => format!("服务{}", state_label(state)),
    }
}

/// 中文状态标签（supervisor `as_str` → 界面词）。
fn state_label(state: &str) -> &'static str {
    match state {
        "stopped" => "已停止",
        "starting" => "启动中",
        "ready" => "运行中",
        "restarting" => "重启中",
        "failed" => "启动失败",
        _ => "未知",
    }
}

/// 创建托盘（启动时调用一次）。
pub fn create(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let status = MenuItem::with_id(app, "status", "服务未知", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &status, &separator, &quit])?;
    let status_item = status.clone();

    TrayIconBuilder::with_id("main-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_string()))?,
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Token Router v2")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => quit_from_tray(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    // 状态刷新：菜单项文本只在创建时算一次，之后 supervisor 状态会变
    // （starting → ready 等）。后台任务定期重写文本；异步锁在这里可用，
    // 不受主线程约束。失败静默：托盘文本过期不是错误。
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let state = handle.state::<crate::AppState>();
            let text = status_text(
                state.supervisor.state().await.as_str(),
                state.supervisor.listen_addr().await.as_deref(),
            );
            let _ = status_item.set_text(text);
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    Ok(())
}

/// 显示并聚焦主窗口。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 托盘「退出」：置真退出标记后走窗口 close——退出守卫（如开启）仍会把
/// 这次关闭交给前端确认；关闭到托盘被绕过（不会只隐藏窗口）。
fn quit_from_tray(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    if let Some(window) = app.get_webview_window("main") {
        let state = app.state::<crate::AppState>();
        state.quit_requested.store(true, Ordering::SeqCst);
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.close();
    }
}

#[cfg(test)]
mod tests {
    use super::{state_label, status_text};

    #[test]
    fn state_label_maps_all_states() {
        assert_eq!(state_label("ready"), "运行中");
        assert_eq!(state_label("stopped"), "已停止");
        assert_eq!(state_label("starting"), "启动中");
        assert_eq!(state_label("restarting"), "重启中");
        assert_eq!(state_label("failed"), "启动失败");
        assert_eq!(state_label("unknown-state"), "未知");
    }

    #[test]
    fn status_text_covers_state_and_listen() {
        assert_eq!(
            status_text("ready", Some("127.0.0.1:17621")),
            "服务运行中 · 127.0.0.1:17621"
        );
        assert_eq!(status_text("ready", None), "服务运行中");
        assert_eq!(status_text("starting", None), "服务启动中");
        assert_eq!(status_text("stopped", Some("127.0.0.1:1")), "服务已停止");
        assert_eq!(status_text("weird", None), "服务未知");
    }
}
