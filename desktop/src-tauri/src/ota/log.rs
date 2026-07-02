use token_router::gateway::logging;
use token_router::gateway::AppConfig;

const TARGET: &str = "token_router::ota";

pub(crate) fn ota_info(message: impl AsRef<str>) {
    write("INFO", message.as_ref());
}

pub(crate) fn ota_warn(message: impl AsRef<str>) {
    write("WARN", message.as_ref());
}

pub(crate) fn ota_error(message: impl AsRef<str>) {
    write("ERROR", message.as_ref());
}

fn write(level: &str, message: &str) {
    if let Ok(config) = AppConfig::load() {
        let _ = logging::append_message(&config.data_dir, level, TARGET, message);
    }
}
