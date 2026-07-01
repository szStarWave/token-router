const DOTENV_PATHS: &[&str] = &["../frontend/.env", "../.env", ".env"];
const ENV_KEY: &str = "VITE_FLOWY_TEST_SERVER";

pub fn is_test_flag_enabled(value: &str) -> bool {
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

fn read_from_dotenv(key: &str) -> Option<String> {
    for path in DOTENV_PATHS {
        let content = std::fs::read_to_string(path).ok()?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (k, value) = line.split_once('=')?;
            if k.trim() == key {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// True when `VITE_FLOWY_TEST_SERVER=1` (env var or frontend/.env).
pub fn flowy_test_server_enabled() -> bool {
    if let Ok(value) = std::env::var(ENV_KEY) {
        if is_test_flag_enabled(&value) {
            return true;
        }
    }
    if let Some(value) = read_from_dotenv(ENV_KEY) {
        if is_test_flag_enabled(&value) {
            return true;
        }
    }
    matches!(option_env!("FLOWY_TEST_SERVER_ENABLED"), Some("true"))
}
