use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use rand::Rng;
use tauri::{AppHandle, Emitter, Manager};
use tauri::State;

use super::apply::spawn_ota_apply_detached;
use super::log::{ota_error, ota_info, ota_warn};
use super::startup_notice::{self, PostOtaRestartNotice, write_post_ota_restart_notice};
use super::updater::{Updater, VersionInfo};
use super::{current_version_string, ota_enabled, ota_temp_dir};

static OTA_STOP: AtomicBool = AtomicBool::new(false);

#[derive(Clone, serde::Serialize)]
pub struct OtaEvent {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

pub struct OtaState {
    inner: Mutex<OtaInner>,
}

struct OtaInner {
    pending: Option<VersionInfo>,
    downloading: bool,
    downloaded_path: Option<String>,
    downloaded_version: Option<VersionInfo>,
    replace_exe: String,
}

impl OtaState {
    fn new() -> Self {
        let replace_exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(OtaInner {
                pending: None,
                downloading: false,
                downloaded_path: None,
                downloaded_version: None,
                replace_exe,
            }),
        }
    }
}

fn emit(app: &AppHandle, message: &str, data: Option<serde_json::Value>) {
    let _ = app.emit(
        "ota:event",
        OtaEvent {
            message: message.to_string(),
            data,
        },
    );
}

fn clone_version(v: &VersionInfo) -> VersionInfo {
    VersionInfo {
        version: v.version.clone(),
        file: v.file.clone(),
        release_notes: v.release_notes.clone(),
    }
}

pub fn start_background_checks(app: AppHandle) {
    if !ota_enabled() {
        ota_info("background check skipped: OTA disabled (non-Windows release build)");
        return;
    }
    ota_info(format!(
        "background check scheduler started ({})",
        super::ota_config_summary()
    ));
    OTA_STOP.store(false, Ordering::SeqCst);
    thread::spawn(move || {
        run_check_once(&app);
        while !OTA_STOP.load(Ordering::SeqCst) {
            let mins = rand::thread_rng().gen_range(10..=60);
            let wait = Duration::from_secs(mins * 60);
            let start = std::time::Instant::now();
            while start.elapsed() < wait && !OTA_STOP.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(5));
            }
            if OTA_STOP.load(Ordering::SeqCst) {
                break;
            }
            run_check_once(&app);
        }
    });
}

pub fn stop_background_checks() {
    OTA_STOP.store(true, Ordering::SeqCst);
    ota_info("background check scheduler stopped");
}

fn run_check_once(app: &AppHandle) {
    ota_info("background check started");
    let state = app.state::<OtaState>();
    let updater = match Updater::new() {
        Ok(u) => u,
        Err(e) => {
            ota_error(format!("background check init failed: {e}"));
            emit(app, "ota.checkFailed", Some(serde_json::json!({ "error": e })));
            return;
        }
    };

    emit(app, "ota.checking", None);

    let version_info = match updater.check_for_update() {
        Ok(v) => v,
        Err(e) => {
            ota_error(format!("background check fetch failed: {e}"));
            emit(app, "ota.checkFailed", Some(serde_json::json!({ "error": e })));
            return;
        }
    };

    let is_newer = match updater.is_newer(&version_info.version) {
        Ok(v) => v,
        Err(e) => {
            ota_error(format!("background check compare failed: {e}"));
            emit(
                app,
                "ota.compareFailed",
                Some(serde_json::json!({ "error": e })),
            );
            return;
        }
    };

    let current = current_version_string();
    if !is_newer {
        if let Ok(mut inner) = state.inner.lock() {
            inner.pending = None;
        }
        ota_info(format!(
            "background check up-to-date: current={current} remote={}",
            version_info.version
        ));
        emit(
            app,
            "ota.upToDate",
            Some(serde_json::json!({
                "current_version": current,
                "remote_version": version_info.version,
            })),
        );
        return;
    }

    if let Ok(mut inner) = state.inner.lock() {
        inner.pending = Some(clone_version(&version_info));
    }

    ota_info(format!(
        "background check found update: current={current} remote={} file={}",
        version_info.version, version_info.file
    ));
    emit(
        app,
        "ota.newVersion",
        Some(serde_json::json!({
            "new_version": version_info.version,
            "file": version_info.file,
            "current_version": current,
        })),
    );
}

#[tauri::command]
pub fn ota_app_version() -> String {
    current_version_string()
}

#[tauri::command]
pub fn ota_check_now(app: AppHandle) -> Result<(), String> {
    if !ota_enabled() {
        return Err("OTA is only available on Windows release builds".into());
    }
    ota_info("manual check requested");
    run_check_once(&app);
    Ok(())
}

#[tauri::command]
pub fn ota_download_update(app: AppHandle, state: State<'_, OtaState>) -> Result<(), String> {
    if !ota_enabled() {
        return Err("OTA is only available on Windows release builds".into());
    }

    let pending = {
        let mut inner = state.inner.lock().map_err(|e| e.to_string())?;
        if inner.downloading {
            ota_warn("download rejected: already in progress");
            return Err("download already in progress".into());
        }
        let pending = inner.pending.clone().ok_or_else(|| {
            ota_warn("download rejected: no pending update");
            "no pending update to download".to_string()
        })?;
        inner.downloading = true;
        pending
    };

    struct DownloadGuard<'a> {
        state: &'a OtaState,
        version: String,
        completed: bool,
    }
    impl Drop for DownloadGuard<'_> {
        fn drop(&mut self) {
            if let Ok(mut inner) = self.state.inner.lock() {
                inner.downloading = false;
            }
            if !self.completed {
                ota_warn(format!(
                    "download session ended without success: version={}",
                    self.version
                ));
            }
        }
    }
    let mut guard = DownloadGuard {
        state: &state,
        version: pending.version.clone(),
        completed: false,
    };

    ota_info(format!(
        "user download requested: current={} target={} file={}",
        current_version_string(),
        pending.version,
        pending.file
    ));

    let updater = match Updater::new() {
        Ok(u) => u,
        Err(e) => {
            ota_error(format!("download init failed: {e}"));
            emit(
                &app,
                "ota.downloadFailed",
                Some(serde_json::json!({ "error": e, "version": pending.version })),
            );
            return Err(e);
        }
    };
    let app_handle = app.clone();
    let app_started = app.clone();
    let version = pending.version.clone();
    let file_path = match updater.download_update(
        &pending,
        |total| {
            emit(
                &app_started,
                "ota.downloadStarted",
                Some(serde_json::json!({ "version": version, "total": total })),
            );
        },
        |progress| {
            emit(
                &app_handle,
                "ota.downloadProgress",
                Some(serde_json::json!({
                    "progress": progress.progress,
                    "downloaded": progress.downloaded,
                    "total": progress.total,
                    "speed_bps": progress.speed_bps,
                })),
            );
        },
    ) {
        Ok(path) => path,
        Err(e) => {
            ota_error(format!("download failed: version={version} reason={e}"));
            emit(
                &app,
                "ota.downloadFailed",
                Some(serde_json::json!({ "error": e, "version": version })),
            );
            return Err(e);
        }
    };

    guard.completed = true;

    emit(
        &app,
        "ota.downloadComplete",
        Some(serde_json::json!({
            "file_path": file_path.to_string_lossy(),
            "version": version,
        })),
    );

    let mut inner = state.inner.lock().map_err(|e| e.to_string())?;
    inner.pending = None;
    inner.downloaded_path = Some(file_path.to_string_lossy().into_owned());
    inner.downloaded_version = Some(pending);
    Ok(())
}

#[tauri::command]
pub fn ota_do_update(app: AppHandle, state: State<'_, OtaState>) -> Result<(), String> {
    if !ota_enabled() {
        return Err("OTA is only available on Windows release builds".into());
    }

    let (dl, downloaded_version, replace_exe) = {
        let inner = state.inner.lock().map_err(|e| e.to_string())?;
        let dl = inner
            .downloaded_path
            .clone()
            .ok_or_else(|| {
                ota_warn("apply rejected: no downloaded package");
                "no downloaded update; call ota_download_update first".to_string()
            })?;
        (
            dl,
            inner.downloaded_version.clone(),
            inner.replace_exe.clone(),
        )
    };

    let version_label = downloaded_version
        .as_ref()
        .map(|v| v.version.as_str())
        .unwrap_or("unknown");

    if !std::path::Path::new(&dl).is_file() {
        let msg = "downloaded file not found".to_string();
        ota_error(format!("apply failed: version={version_label} reason={msg} path={dl}"));
        return Err(msg);
    }
    if replace_exe.is_empty() {
        let msg = "replaceExe is not configured".to_string();
        ota_error(format!("apply failed: version={version_label} reason={msg}"));
        return Err(msg);
    }

    ota_info(format!(
        "apply started: version={version_label} setup={dl} app_exe={replace_exe}"
    ));

    emit(
        &app,
        "ota.updateApplyStarted",
        Some(serde_json::json!({ "target": replace_exe, "version": version_label })),
    );

    if let Some(v) = downloaded_version.as_ref() {
        let data_dir = startup_notice::data_dir()?;
        let notice = PostOtaRestartNotice {
            show: true,
            version: v.version.clone(),
            release_notes: v.release_notes.clone(),
        };
        if let Err(e) = write_post_ota_restart_notice(&data_dir, &notice) {
            ota_warn(format!(
                "apply post-restart notice write failed: version={} reason={e}",
                v.version
            ));
        }
    }

    let self_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let temp_dir = ota_temp_dir()?;

    if let Err(e) = spawn_ota_apply_detached(
        &self_exe.to_string_lossy(),
        &replace_exe,
        &dl,
        &temp_dir.to_string_lossy(),
    ) {
        ota_error(format!(
            "apply failed: version={version_label} target={replace_exe} reason={e}"
        ));
        emit(
            &app,
            "ota.updateApplyFailed",
            Some(serde_json::json!({ "error": e, "version": version_label })),
        );
        return Err(e);
    }

    if let Ok(mut inner) = state.inner.lock() {
        inner.downloaded_path = None;
        inner.downloaded_version = None;
    }

    ota_info(format!(
        "apply spawned successfully: version={version_label} setup={dl}; exiting app"
    ));
    emit(
        &app,
        "ota.updateApplyComplete",
        Some(serde_json::json!({ "target": replace_exe, "version": version_label })),
    );

    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn ota_get_post_restart_notice() -> Result<PostOtaRestartNotice, String> {
    let data_dir = startup_notice::data_dir()?;
    startup_notice::read_and_consume_post_ota_restart_notice(&data_dir)
}

pub fn manage_state() -> OtaState {
    OtaState::new()
}
