fn is_test_flag_enabled(value: &str) -> bool {
    let n = value.trim().trim_matches('"').trim_matches('\'').to_lowercase();
    if n.is_empty() {
        return false;
    }
    !matches!(n.as_str(), "0" | "false" | "off" | "no")
}

fn read_flowy_test_server_from_dotenv() -> Option<String> {
    for path in [std::path::Path::new("../.env"), std::path::Path::new(".env")] {
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
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "VITE_FLOWY_TEST_SERVER" {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

fn flowy_test_server_enabled() -> bool {
    if let Ok(value) = std::env::var("VITE_FLOWY_TEST_SERVER") {
        return is_test_flag_enabled(&value);
    }
    if let Some(value) = read_flowy_test_server_from_dotenv() {
        return is_test_flag_enabled(&value);
    }
    false
}

fn main() {
    let enabled = flowy_test_server_enabled();
    println!("cargo:rustc-env=FLOWY_TEST_SERVER_ENABLED={}", enabled);
    println!("cargo:rerun-if-changed=../.env");
    println!("cargo:rerun-if-env-changed=VITE_FLOWY_TEST_SERVER");

    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "gateway_start",
                "gateway_stop",
                "gateway_restart",
                "gateway_is_running",
                "gateway_url",
                "gateway_status",
                "show_main_window",
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}
