fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                // supervisor（服务生命周期）
                "supervisor_status",
                "supervisor_start",
                "supervisor_stop",
                "supervisor_restart",
                // admin 代理（token 不进 WebView）
                "admin_request",
                // host 能力
                "host_app_info",
                "host_open_path",
                "host_ui_state_load",
                "host_ui_state_save",
                // Agent onboarding（§8）
                "onboard_detect",
                "onboard_plan",
                "onboard_apply",
                "onboard_verify",
                "onboard_wsl_detect",
                // SSE 事件桥（§6.3）
                "admin_events_subscribe",
                "admin_events_unsubscribe",
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}
