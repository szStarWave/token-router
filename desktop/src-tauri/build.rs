fn is_test_flag_enabled(value: &str) -> bool {
    let n = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_lowercase();
    if n.is_empty() {
        return false;
    }
    !matches!(n.as_str(), "0" | "false" | "off" | "no")
}

fn read_dotenv_key(key: &str) -> Option<String> {
    for path in [
        std::path::Path::new("../frontend/.env"),
        std::path::Path::new("../.env"),
        std::path::Path::new(".env"),
    ] {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((k, value)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

fn read_env_key(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .or_else(|| read_dotenv_key(key))
}

fn flowy_test_server_enabled() -> bool {
    #[cfg(not(debug_assertions))]
    {
        return false;
    }
    read_env_key("VITE_FLOWY_TEST_SERVER")
        .map(|v| is_test_flag_enabled(&v))
        .unwrap_or(false)
}

fn ota_region_scope() -> &'static str {
    let edition = read_env_key("VITE_EDITION").unwrap_or_else(|| "domestic".to_string());
    let edition = edition
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_lowercase();
    if edition == "international" {
        "INTL"
    } else {
        "CN"
    }
}

fn main() {
    let enabled = flowy_test_server_enabled();
    println!("cargo:rustc-env=FLOWY_TEST_SERVER_ENABLED={enabled}");
    println!("cargo:rustc-env=OTA_BASE_URL=https://modelscope.cn/datasets/flowy2025/token_router_versions/resolve/master");
    println!("cargo:rustc-env=OTA_REGION_SCOPE={}", ota_region_scope());
    println!("cargo:rustc-env=OTA_CHANNEL=flowy");
    println!("cargo:rustc-env=OTA_WITH_ACCOUNT=true");
    println!("cargo:rerun-if-changed=../frontend/.env");
    println!("cargo:rerun-if-changed=../.env");
    println!("cargo:rerun-if-changed=.env");
    println!("cargo:rerun-if-env-changed=VITE_FLOWY_TEST_SERVER");
    println!("cargo:rerun-if-env-changed=VITE_EDITION");

    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "gateway_start",
                "gateway_stop",
                "gateway_restart",
                "gateway_is_running",
                "gateway_url",
                "gateway_status",
                "gateway_read_logs",
                "gateway_read_routing_logs",
                "gateway_open_logs_dir",
                "show_main_window",
                "hide_main_window",
                "herdsman_open_or_install",
                "herdsman_get_status",
                "herdsman_start",
                "feedback_app_version",
                "feedback_submit",
                "ota_app_version",
                "ota_check_now",
                "ota_download_update",
                "ota_do_update",
                "ota_get_post_restart_notice",
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}
