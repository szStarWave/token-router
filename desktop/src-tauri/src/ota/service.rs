use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use rand::Rng;
use tauri::{AppHandle, Emitter, Manager};
use tauri::State;

use super::apply::spawn_ota_apply_detached;
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
        return;
    }
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
}

fn run_check_once(app: &AppHandle) {
    let state = app.state::<OtaState>();
    let updater = match Updater::new() {
        Ok(u) => u,
        Err(e) => {
            emit(app, "ota.checkFailed", Some(serde_json::json!({ "error": e })));
            return;
        }
    };

    emit(app, "ota.checking", None);

    let version_info = match updater.check_for_update() {
        Ok(v) => v,
        Err(e) => {
            emit(app, "ota.checkFailed", Some(serde_json::json!({ "error": e })));
            return;
        }
    };

    let is_newer = match updater.is_newer(&version_info.version) {
        Ok(v) => v,
        Err(e) => {
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
            return Err("download already in progress".into());
        }
        let pending = inner.pending.clone().ok_or("no pending update to download")?;
        inner.downloading = true;
        pending
    };

    struct DownloadGuard<'a> {
        state: &'a OtaState,
    }
    impl Drop for DownloadGuard<'_> {
        fn drop(&mut self) {
            if let Ok(mut inner) = self.state.inner.lock() {
                inner.downloading = false;
            }
        }
    }
    let _guard = DownloadGuard { state: &state };

    emit(
        &app,
        "ota.downloadStarted",
        Some(serde_json::json!({ "version": pending.version })),
    );

    let updater = Updater::new()?;
    let app_handle = app.clone();
    let version = pending.version.clone();
    let file_path = updater.download_update(&pending, |percent| {
        emit(
            &app_handle,
            "ota.downloadProgress",
            Some(serde_json::json!({ "progress": percent })),
        );
    })?;

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
            .ok_or("no downloaded update; call ota_download_update first")?;
        (
            dl,
            inner.downloaded_version.clone(),
            inner.replace_exe.clone(),
        )
    };

    if !std::path::Path::new(&dl).is_file() {
        return Err("downloaded file not found".into());
    }
    if replace_exe.is_empty() {
        return Err("replaceExe is not configured".into());
    }

    emit(
        &app,
        "ota.updateApplyStarted",
        Some(serde_json::json!({ "target": replace_exe })),
    );

    if let Some(v) = downloaded_version.as_ref() {
        let data_dir = startup_notice::data_dir()?;
        let notice = PostOtaRestartNotice {
            show: true,
            version: v.version.clone(),
            release_notes: v.release_notes.clone(),
        };
        let _ = write_post_ota_restart_notice(&data_dir, &notice);
    }

    let self_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let temp_dir = ota_temp_dir()?;

    if let Err(e) = spawn_ota_apply_detached(
        &self_exe.to_string_lossy(),
        &replace_exe,
        &dl,
        &temp_dir.to_string_lossy(),
    ) {
        emit(
            &app,
            "ota.updateApplyFailed",
            Some(serde_json::json!({ "error": e })),
        );
        return Err(e);
    }

    if let Ok(mut inner) = state.inner.lock() {
        inner.downloaded_path = None;
        inner.downloaded_version = None;
    }

    emit(
        &app,
        "ota.updateApplyComplete",
        Some(serde_json::json!({ "target": replace_exe })),
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
