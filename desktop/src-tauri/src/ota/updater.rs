use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, REFERER, USER_AGENT};
use semver::Version;
use serde::Deserialize;

use super::log::{ota_error, ota_info};
use super::{build_download_url, build_ota_manifest_url, current_version_string, format_semver_with_build, ota_temp_dir};

const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(200);
const MODELSCOPE_REFERER: &str = "https://modelscope.cn/";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub progress: i32,
    pub speed_bps: u64,
}

impl DownloadProgress {
    fn new(downloaded: u64, total: u64, speed_bps: u64) -> Self {
        let progress = if total > 0 {
            ((downloaded as f64 / total as f64) * 100.0).round() as i32
        } else {
            -1
        };
        Self {
            downloaded,
            total,
            progress: progress.clamp(0, 100),
            speed_bps,
        }
    }
}

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
        let client = build_ota_client()?;
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
        ota_info(format!(
            "check update: current={} manifest_url={url}",
            current_version_string()
        ));

        let resp = self
            .client
            .get(&url)
            .headers(ota_request_headers())
            .send()
            .map_err(|e| format!("failed to fetch version info: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let detail = response_error_detail(resp);
            let msg = format!("manifest HTTP {status}{detail}");
            ota_error(format!("check update failed: {msg} url={url}"));
            return Err(msg);
        }

        let version_info = resp
            .json::<VersionInfo>()
            .map_err(|e| format!("failed to parse version info: {e}"))?;
        ota_info(format!(
            "check update: remote_version={} file={} url={url}",
            version_info.version, version_info.file
        ));
        Ok(version_info)
    }

    pub fn is_newer(&self, version: &str) -> Result<bool, String> {
        let new_ver = Version::parse(&normalize_version_tag(version))
            .map_err(|e| format!("failed to parse new version {version}: {e}"))?;
        Ok(new_ver > self.current_ver)
    }

    pub fn download_update(
        &self,
        version_info: &VersionInfo,
        mut on_started: impl FnMut(u64),
        mut on_progress: impl FnMut(DownloadProgress),
    ) -> Result<PathBuf, String> {
        let download_url = build_download_url(&version_info.file);
        let temp_dir = ota_temp_dir()?;
        let file_path = temp_dir.join(&version_info.file);

        ota_info(format!(
            "download started: current={} target_version={} file={} url={download_url} dest={}",
            current_version_string(),
            version_info.version,
            version_info.file,
            file_path.display()
        ));

        let mut resp = self
            .client
            .get(&download_url)
            .headers(ota_request_headers())
            .send()
            .map_err(|e| {
                let msg = format!("download request failed: {e}");
                ota_error(format!("{msg} url={download_url}"));
                msg
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let detail = response_error_detail(resp);
            let msg = format!("download HTTP {status}{detail}");
            ota_error(format!(
                "{msg} url={download_url} version={} file={}",
                version_info.version, version_info.file
            ));
            return Err(msg);
        }

        let total = resp.content_length().unwrap_or(0);
        on_started(total);
        on_progress(DownloadProgress::new(0, total, 0));
        ota_info(format!(
            "download response ok: version={} total_bytes={total} url={download_url}",
            version_info.version
        ));

        let mut file = std::fs::File::create(&file_path).map_err(|e| {
            let msg = e.to_string();
            ota_error(format!(
                "download create file failed: {msg} dest={}",
                file_path.display()
            ));
            msg
        })?;
        let mut downloaded: u64 = 0;
        let mut speed_bps: u64 = 0;
        let mut last_pct = -1i32;
        let mut last_emit = Instant::now();
        let mut last_speed_sample = Instant::now();
        let mut last_speed_downloaded: u64 = 0;
        let mut buf = [0u8; 8192];

        loop {
            let n = match resp.read(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    let msg = e.to_string();
                    ota_error(format!(
                        "download interrupted: version={} downloaded={downloaded}/{total} reason={msg} url={download_url}",
                        version_info.version
                    ));
                    return Err(msg);
                }
            };
            if n == 0 {
                break;
            }
            if let Err(e) = file.write_all(&buf[..n]) {
                let msg = e.to_string();
                ota_error(format!(
                    "download write failed: version={} downloaded={downloaded}/{total} reason={msg} dest={}",
                    version_info.version,
                    file_path.display()
                ));
                return Err(msg);
            }
            downloaded += n as u64;

            let now = Instant::now();
            if now.duration_since(last_speed_sample) >= PROGRESS_EMIT_INTERVAL {
                let elapsed = now.duration_since(last_speed_sample).as_secs_f64();
                if elapsed > 0.0 {
                    speed_bps = ((downloaded.saturating_sub(last_speed_downloaded)) as f64 / elapsed)
                        .round() as u64;
                }
                last_speed_sample = now;
                last_speed_downloaded = downloaded;
            }

            let snapshot = DownloadProgress::new(downloaded, total, speed_bps);
            let pct = snapshot.progress;
            let should_emit = now.duration_since(last_emit) >= PROGRESS_EMIT_INTERVAL
                || (total > 0 && pct != last_pct);
            if should_emit {
                last_emit = now;
                if total > 0 {
                    last_pct = pct;
                }
                on_progress(snapshot);
            }
        }

        if last_pct < 100 {
            on_progress(DownloadProgress::new(downloaded, total.max(downloaded), speed_bps));
        }

        ota_info(format!(
            "download complete: version={} bytes={downloaded} dest={}",
            version_info.version,
            file_path.display()
        ));
        Ok(file_path)
    }
}

fn build_ota_client() -> Result<Client, String> {
    let user_agent = format!("Token-Router/{}/ota", current_version_string());
    Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .cookie_store(true)
        .user_agent(user_agent)
        .build()
        .map_err(|e| e.to_string())
}

fn ota_request_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&format!("Token-Router/{}/ota", current_version_string()))
            .unwrap_or_else(|_| HeaderValue::from_static("Token-Router/ota")),
    );
    headers.insert(REFERER, HeaderValue::from_static(MODELSCOPE_REFERER));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers
}

fn response_error_detail(resp: reqwest::blocking::Response) -> String {
    let body = resp.text().unwrap_or_default();
    let preview: String = body.chars().take(200).collect();
    if preview.is_empty() {
        String::new()
    } else {
        format!(" body={preview:?}")
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

    ota_info(format!(
        "apply cli started: target={target} package={package} temp_dir={temp_dir}"
    ));

    #[cfg(windows)]
    {
        if let Err(e) = super::apply::windows::run_apply(&target, &package, &temp_dir) {
            ota_error(format!("apply cli failed: {e}"));
            eprintln!("{e}");
            return 1;
        }
        ota_info("apply cli complete");
        return 0;
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(e) = super::apply::macos::run_apply(&target, &package, &temp_dir) {
            ota_error(format!("apply cli failed: {e}"));
            eprintln!("{e}");
            return 1;
        }
        ota_info("apply cli complete");
        return 0;
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (target, package, temp_dir);
        ota_error("apply cli failed: OTA apply is not supported on this platform");
        eprintln!("OTA apply is not supported on this platform");
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
