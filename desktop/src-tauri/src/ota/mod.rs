pub mod apply;
pub mod log;
pub mod service;
pub mod startup_notice;
mod updater;

pub use service::{
    ota_app_version, ota_check_now, ota_do_update, ota_download_update, ota_get_post_restart_notice,
    start_background_checks, stop_background_checks, OtaEvent,
};
pub use startup_notice::PostOtaRestartNotice;
pub use updater::run_ota_apply_cli;

const OTA_BASE_URL: &str = env!("OTA_BASE_URL");
const OTA_REGION_SCOPE: &str = env!("OTA_REGION_SCOPE");
const OTA_CHANNEL: &str = env!("OTA_CHANNEL");
const OTA_WITH_ACCOUNT: &str = env!("OTA_WITH_ACCOUNT");

pub fn ota_enabled() -> bool {
    cfg!(windows) && !cfg!(debug_assertions)
}

pub fn ota_config_summary() -> String {
    format!(
        "base={OTA_BASE_URL} region={OTA_REGION_SCOPE} channel={OTA_CHANNEL} with_account={OTA_WITH_ACCOUNT}"
    )
}

pub fn build_ota_manifest_url() -> String {
    build_ota_url("latest.json")
}

pub fn build_download_url(file_name: &str) -> String {
    build_ota_url(file_name)
}

fn build_ota_url(file_name: &str) -> String {
    let account_dir = if OTA_WITH_ACCOUNT == "true" {
        "with_account"
    } else {
        "without_account"
    };
    format!(
        "{OTA_BASE_URL}/{OTA_REGION_SCOPE}/{OTA_CHANNEL}/{account_dir}/{file_name}"
    )
}

/// Convert git-describe style `v0.1.6-10-g083d26b` to semver `v0.1.6-10+g083d26b`.
pub fn format_semver_with_build(tag: &str) -> String {
    if let Some(idx) = tag.rfind("-g") {
        let suffix = &tag[idx + 1..];
        if suffix.len() > 1 && suffix[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            return format!("{}+{}", &tag[..idx], &tag[idx + 1..]);
        }
    }
    tag.to_string()
}

pub fn current_version_string() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn ota_temp_dir() -> Result<std::path::PathBuf, String> {
    let config = token_router::gateway::AppConfig::load().map_err(|e| e.to_string())?;
    let dir = config.data_dir.join("ota-temp");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::format_semver_with_build;

    #[test]
    fn format_semver_with_build_replaces_git_suffix() {
        assert_eq!(
            format_semver_with_build("v0.1.6-10-g083d26b"),
            "v0.1.6-10+g083d26b"
        );
    }
}
