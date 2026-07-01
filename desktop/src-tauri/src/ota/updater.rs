use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;

use super::{build_download_url, build_ota_manifest_url, current_version_string, format_semver_with_build, ota_temp_dir};

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub file: String,
    #[serde(default)]
    pub release_notes: std::collections::HashMap<String, Vec<String>>,
}

pub struct Updater {
    current_ver: Version,
    client: Client,
}

impl Updater {
    pub fn new() -> Result<Self, String> {
        let current = current_version_string();
        let parsed = Version::parse(&normalize_version_tag(&current))
            .map_err(|e| format!("failed to parse current version {current}: {e}"))?;
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            current_ver: parsed,
            client,
        })
    }

    pub fn current_version(&self) -> &Version {
        &self.current_ver
    }

    pub fn check_for_update(&self) -> Result<VersionInfo, String> {
        let url = build_ota_manifest_url();
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| format!("failed to fetch version info: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("unexpected status code: {}", resp.status()));
        }
        resp.json::<VersionInfo>()
            .map_err(|e| format!("failed to parse version info: {e}"))
    }

    pub fn is_newer(&self, version: &str) -> Result<bool, String> {
        let new_ver = Version::parse(&normalize_version_tag(version))
            .map_err(|e| format!("failed to parse new version {version}: {e}"))?;
        Ok(new_ver > self.current_ver)
    }

    pub fn download_update(
        &self,
        version_info: &VersionInfo,
        on_progress: impl Fn(i32),
    ) -> Result<PathBuf, String> {
        let download_url = build_download_url(&version_info.file);
        let temp_dir = ota_temp_dir()?;
        let file_path = temp_dir.join(&version_info.file);

        let mut resp = self
            .client
            .get(&download_url)
            .send()
            .map_err(|e| format!("download request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download HTTP {}", resp.status()));
        }

        let total = resp.content_length().unwrap_or(0);
        let mut file = std::fs::File::create(&file_path).map_err(|e| e.to_string())?;
        let mut downloaded: u64 = 0;
        let mut last_pct = -1i32;
        let mut buf = [0u8; 8192];

        loop {
            let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            downloaded += n as u64;
            if total > 0 {
                let pct = ((downloaded as f64 / total as f64) * 100.0).round() as i32;
                let pct = pct.clamp(0, 100);
                if pct != last_pct {
                    last_pct = pct;
                    on_progress(pct);
                }
            }
        }
        if last_pct < 100 {
            on_progress(100);
        }
        Ok(file_path)
    }
}

fn normalize_version_tag(tag: &str) -> String {
    let formatted = format_semver_with_build(tag.trim());
    formatted.trim_start_matches('v').to_string()
}

pub fn run_ota_apply_cli(args: &[String]) -> i32 {
    if args.len() < 2 || args[0] != "ota" || args[1] != "apply" {
        eprintln!("usage: ota apply --target <exe> --package <path> [--temp-dir <dir>]");
        return 1;
    }

    let mut target = String::new();
    let mut package = String::new();
    let mut temp_dir = String::new();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--target" if i + 1 < args.len() => {
                target = args[i + 1].clone();
                i += 2;
            }
            "--package" if i + 1 < args.len() => {
                package = args[i + 1].clone();
                i += 2;
            }
            "--temp-dir" if i + 1 < args.len() => {
                temp_dir = args[i + 1].clone();
                i += 2;
            }
            _ => {
                eprintln!("unknown arg: {}", args[i]);
                return 1;
            }
        }
    }

    if target.is_empty() || package.is_empty() {
        eprintln!("--target and --package are required");
        return 1;
    }

    if temp_dir.is_empty() {
        temp_dir = ota_temp_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    }

    #[cfg(windows)]
    {
        if let Err(e) = super::apply::windows::run_apply(&target, &package, &temp_dir) {
            eprintln!("{e}");
            return 1;
        }
        return 0;
    }

    #[cfg(not(windows))]
    {
        let _ = (target, package, temp_dir);
        eprintln!("OTA apply is only supported on Windows");
        1
    }
}

pub fn same_exe_path(a: &str, b: &str) -> bool {
    let a = std::fs::canonicalize(a).ok();
    let b = std::fs::canonicalize(b).ok();
    match (a, b) {
        (Some(aa), Some(bb)) => aa == bb,
        _ => false,
    }
}

pub fn package_exists(path: &Path) -> bool {
    path.is_file()
}
