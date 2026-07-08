use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use token_router::config::normalize_client_http_url;
use token_router::gateway::logging;
use token_router::gateway::AppConfig;

const PIPE_NAME: &str = r"\\.\pipe\Herdsman-status";
const LOG_TARGET: &str = "token_router::herdsman";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const MODEL_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DISCOVER_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_INSTALL_URL: &str = "https://flowyaipc.cn/#ai-engine";
const DEFAULT_HTTP_PORTS: &[u16] = &[8080, 11434, 8081, 8000];

static SERVICE_RUNNING: AtomicBool = AtomicBool::new(false);
static PROBE_NOW: AtomicBool = AtomicBool::new(false);
static RUNTIME_STATE: OnceLock<Arc<Mutex<HerdsmanRuntimeState>>> = OnceLock::new();

#[derive(Clone, Debug, Default)]
struct HerdsmanRuntimeState {
    connected: bool,
    endpoint: Option<String>,
    openai_endpoint: Option<String>,
    models: Vec<HerdsmanModelInfo>,
    installed: bool,
    launcher_path: Option<String>,
}

#[derive(Clone, Serialize, Default)]
pub struct HerdsmanStatusSnapshot {
    pub connected: bool,
    pub endpoint: Option<String>,
    pub openai_endpoint: Option<String>,
    pub models: Vec<HerdsmanModelInfo>,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launcher_path: Option<String>,
}

#[derive(Clone, Serialize)]
struct HerdsmanInstallDetected {
    installed: bool,
    launcher_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct HerdsmanStatus {
    #[allow(dead_code)]
    app_name: Option<String>,
    host: Option<String>,
    port: Option<u32>,
    endpoint: String,
    #[allow(dead_code)]
    webui_url: Option<String>,
    openai_endpoint: String,
    #[allow(dead_code)]
    timestamp: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct RuntimeInfo {
    context_size: Option<u64>,
    #[allow(dead_code)]
    port: Option<u32>,
    #[allow(dead_code)]
    inference_engine: Option<String>,
    run_status: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawHerdsmanModel {
    id: Option<String>,
    name: String,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    runtime_info: Option<RuntimeInfo>,
}

#[derive(Clone, Serialize, Debug)]
pub struct HerdsmanModelInfo {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub source: &'static str,
}

#[derive(Clone, Serialize)]
pub struct HerdsmanOpenResult {
    pub opened: String,
    pub target: String,
}

fn herdsman_log(level: &str, message: impl AsRef<str>) {
    if let Ok(config) = AppConfig::load() {
        let _ = logging::append_message(&config.data_dir, level, LOG_TARGET, message.as_ref());
    }
}

fn herdsman_info(message: impl AsRef<str>) {
    herdsman_log("INFO", message);
}

fn herdsman_warn(message: impl AsRef<str>) {
    herdsman_log("WARN", message);
}

fn runtime_state() -> Arc<Mutex<HerdsmanRuntimeState>> {
    RUNTIME_STATE
        .get_or_init(|| Arc::new(Mutex::new(HerdsmanRuntimeState::default())))
        .clone()
}

fn snapshot_from_state(state: &HerdsmanRuntimeState) -> HerdsmanStatusSnapshot {
    HerdsmanStatusSnapshot {
        connected: state.connected,
        endpoint: state.endpoint.clone(),
        openai_endpoint: state.openai_endpoint.clone(),
        models: state.models.clone(),
        installed: state.installed,
        launcher_path: state.launcher_path.clone(),
    }
}

fn update_runtime_state<F>(update: F)
where
    F: FnOnce(&mut HerdsmanRuntimeState),
{
    if let Ok(mut guard) = runtime_state().lock() {
        update(&mut guard);
    }
}

fn is_running_model(model: &RawHerdsmanModel) -> bool {
    let model_type = model.r#type.as_deref().unwrap_or("");
    let status = model.status.as_deref().unwrap_or("");
    let run_status = model
        .runtime_info
        .as_ref()
        .and_then(|r| r.run_status.as_deref())
        .unwrap_or("");
    (model_type == "multimodal" || model_type == "text-generation")
        && status == "installed"
        && run_status == "running"
}

fn map_models(raw: Vec<RawHerdsmanModel>, openai_endpoint: &str) -> Vec<HerdsmanModelInfo> {
    raw.into_iter()
        .filter(is_running_model)
        .map(|model| HerdsmanModelInfo {
            id: model.name.clone(),
            name: model.name,
            endpoint: openai_endpoint.to_string(),
            context_window: model.runtime_info.and_then(|r| r.context_size),
            icon: model.icon,
            source: "herdsman",
        })
        .collect()
}

fn models_signature(models: &[HerdsmanModelInfo]) -> String {
    models
        .iter()
        .map(|m| {
            if let Some(ctx) = m.context_window {
                format!("{}@{}", m.name, ctx)
            } else {
                m.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn log_running_models(models: &[HerdsmanModelInfo]) {
    if models.is_empty() {
        herdsman_info("running models: none");
        return;
    }
    let details: Vec<String> = models
        .iter()
        .map(|m| {
            if let Some(ctx) = m.context_window {
                format!("{} (context={ctx})", m.name)
            } else {
                m.name.clone()
            }
        })
        .collect();
    herdsman_info(format!(
        "running models ({}): {}",
        models.len(),
        details.join(", ")
    ));
}

#[cfg(windows)]
fn pipe_name_wide() -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    OsStr::new(PIPE_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
type PipeHandle = windows::Win32::Foundation::HANDLE;

#[cfg(not(windows))]
type PipeHandle = ();

#[cfg(windows)]
fn connect_pipe() -> Option<PipeHandle> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, OPEN_EXISTING,
    };

    let wide = pipe_name_wide();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            Default::default(),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };

    match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => Some(h),
        _ => None,
    }
}

#[cfg(not(windows))]
fn connect_pipe() -> Option<PipeHandle> {
    None
}

#[cfg(windows)]
fn read_pipe_message(handle: PipeHandle) -> Option<String> {
    use windows::Win32::Foundation::ERROR_MORE_DATA;
    use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
    use windows::Win32::System::Pipes::{SetNamedPipeHandleState, PIPE_READMODE_MESSAGE};

    unsafe {
        let mut mode = PIPE_READMODE_MESSAGE;
        let _ = SetNamedPipeHandleState(handle, Some(&mut mode), None, None);
    }

    let mut written = 0u32;
    unsafe {
        WriteFile(handle, Some(b"/status"), Some(&mut written), None).ok()?;
    }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let mut read = 0u32;
        match unsafe { ReadFile(handle, Some(&mut chunk), Some(&mut read), None) } {
            Ok(()) => {
                if read > 0 {
                    buf.extend_from_slice(&chunk[..read as usize]);
                }
                break;
            }
            Err(e) if e.code().0 as u32 == ERROR_MORE_DATA.0 => {
                if read > 0 {
                    buf.extend_from_slice(&chunk[..read as usize]);
                }
                continue;
            }
            Err(_) => return None,
        }
    }

    let response = String::from_utf8_lossy(&buf).trim().to_string();
    if response.is_empty() {
        None
    } else {
        Some(response)
    }
}

#[cfg(windows)]
struct SendPipeHandle(PipeHandle);

#[cfg(windows)]
unsafe impl Send for SendPipeHandle {}

#[cfg(windows)]
fn duplicate_pipe_handle(handle: PipeHandle) -> Option<PipeHandle> {
    use windows::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut dup = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut dup,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .ok()
        .map(|_| dup)
    }
}

#[cfg(windows)]
fn watch_pipe_disconnect(watch: SendPipeHandle, alive: Arc<AtomicBool>) {
    let handle = watch.0;
    use windows::Win32::Foundation::{CloseHandle, ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED};
    use windows::Win32::Storage::FileSystem::ReadFile;

    let mut buf = [0u8; 256];
    loop {
        let mut read = 0u32;
        let result = unsafe { ReadFile(handle, Some(&mut buf), Some(&mut read), None) };
        match result {
            Ok(()) if read == 0 => break,
            Err(e) => {
                let code = e.code().0 as u32;
                if code == ERROR_BROKEN_PIPE.0 || code == ERROR_PIPE_NOT_CONNECTED.0 {
                    break;
                }
                break;
            }
            Ok(()) => continue,
        }
    }
    alive.store(false, Ordering::SeqCst);
    unsafe {
        let _ = CloseHandle(handle);
    }
}

#[cfg(windows)]
fn close_pipe(handle: PipeHandle) {
    use windows::Win32::Foundation::CloseHandle;

    unsafe {
        let _ = CloseHandle(handle);
    }
}

#[cfg(windows)]
fn pipe_is_available() -> bool {
    if let Some(handle) = connect_pipe() {
        close_pipe(handle);
        true
    } else {
        false
    }
}

#[cfg(not(windows))]
fn pipe_is_available() -> bool {
    false
}

fn read_status_from_pipe() -> Option<HerdsmanStatus> {
    #[cfg(windows)]
    {
        let handle = connect_pipe()?;
        let response = read_pipe_message(handle)?;
        close_pipe(handle);
        return serde_json::from_str(&response).ok();
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn push_api_base_candidate(candidates: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim().trim_end_matches('/').trim_end_matches("/v1");
    if trimmed.is_empty() {
        return;
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    if !candidates.iter().any(|c| c == &with_scheme) {
        candidates.push(with_scheme);
    }
    let normalized = normalize_client_http_url(trimmed)
        .trim_end_matches('/')
        .to_string();
    if !normalized.is_empty() && !candidates.iter().any(|c| c == &normalized) {
        candidates.push(normalized);
    }
}

fn fetch_models(endpoint: &str, openai_endpoint: &str) -> Vec<HerdsmanModelInfo> {
    let client_openai_endpoint = normalize_client_http_url(openai_endpoint);
    let mut candidates = Vec::new();
    push_api_base_candidate(&mut candidates, endpoint);
    push_api_base_candidate(&mut candidates, openai_endpoint);
    fetch_models_from_candidates(&candidates, &client_openai_endpoint)
}

fn fetch_models_from_status(status: &HerdsmanStatus) -> Vec<HerdsmanModelInfo> {
    let client_openai_endpoint = normalize_client_http_url(&status.openai_endpoint);
    let mut candidates = Vec::new();
    push_api_base_candidate(&mut candidates, &status.endpoint);
    push_api_base_candidate(&mut candidates, &status.openai_endpoint);
    if let Some(port) = status.port {
        push_api_base_candidate(&mut candidates, &format!("http://127.0.0.1:{port}"));
        if let Some(host) = status.host.as_deref() {
            push_api_base_candidate(&mut candidates, &format!("http://{host}:{port}"));
        }
    }
    fetch_models_from_candidates(&candidates, &client_openai_endpoint)
}

fn fetch_models_from_candidates(
    candidates: &[String],
    openai_endpoint: &str,
) -> Vec<HerdsmanModelInfo> {
    let mut last_empty = Vec::new();
    for (idx, base) in candidates.iter().enumerate() {
        match try_fetch_models(base, openai_endpoint) {
            Some(models) if !models.is_empty() => return models,
            Some(models) => {
                last_empty = models;
                if idx + 1 == candidates.len() {
                    return last_empty;
                }
            }
            None => continue,
        }
    }
    last_empty
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ModelsApiResponse {
    Direct(Vec<RawHerdsmanModel>),
    Data { data: Vec<RawHerdsmanModel> },
    Models { models: Vec<RawHerdsmanModel> },
}

fn parse_models_response(body: &str) -> Option<Vec<RawHerdsmanModel>> {
    if let Ok(models) = serde_json::from_str::<Vec<RawHerdsmanModel>>(body) {
        return Some(models);
    }
    if let Ok(wrapped) = serde_json::from_str::<ModelsApiResponse>(body) {
        return Some(match wrapped {
            ModelsApiResponse::Direct(models) => models,
            ModelsApiResponse::Data { data } => data,
            ModelsApiResponse::Models { models } => models,
        });
    }
    None
}

fn try_fetch_models(base: &str, openai_endpoint: &str) -> Option<Vec<HerdsmanModelInfo>> {
    let url = format!("{}/api/v1/models", base.trim_end_matches('/'));
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            herdsman_warn(format!("models fetch client build failed: {e}"));
            return None;
        }
    };

    let response = match client.get(&url).send() {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            herdsman_warn(format!(
                "models fetch failed: url={url} status={}",
                r.status()
            ));
            return None;
        }
        Err(e) => {
            herdsman_warn(format!("models fetch error: url={url} err={e}"));
            return None;
        }
    };

    let body = match response.text() {
        Ok(text) => text,
        Err(e) => {
            herdsman_warn(format!("models fetch read failed: url={url} err={e}"));
            return None;
        }
    };

    let raw = parse_models_response(&body)?;
    let models = map_models(raw.clone(), openai_endpoint);
    if raw.is_empty() {
        herdsman_info(format!("models fetch parsed empty list from {url}"));
    } else if models.is_empty() {
        herdsman_warn(format!(
            "models fetch: {} raw models from {url} but none marked running",
            raw.len()
        ));
    } else {
        herdsman_info(format!("models fetch ok: url={url} count={}", models.len()));
    }
    Some(models)
}

fn emit_connected(app: &AppHandle, connected: bool) {
    let _ = app.emit("herdsman-connected", connected);
}

fn emit_models(app: &AppHandle, models: &[HerdsmanModelInfo]) {
    let _ = app.emit("herdsman-models", models.to_vec());
}

fn emit_install_detected(app: &AppHandle, installed: bool, launcher_path: Option<String>) {
    let _ = app.emit(
        "herdsman-install-detected",
        HerdsmanInstallDetected {
            installed,
            launcher_path,
        },
    );
}

fn request_immediate_probe() {
    PROBE_NOW.store(true, Ordering::SeqCst);
}

fn sleep_interruptible(duration: Duration) {
    let until = std::time::Instant::now() + duration;
    while SERVICE_RUNNING.load(Ordering::SeqCst) && std::time::Instant::now() < until {
        if PROBE_NOW.swap(false, Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn derive_openai_endpoint(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    let with_v1 = if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    };
    normalize_client_http_url(&with_v1)
}

#[cfg(windows)]
fn read_herdsman_config_candidates(candidates: &mut Vec<String>) {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let profile = match env::var("USERPROFILE") {
        Ok(p) => p,
        Err(_) => return,
    };
    let dir = PathBuf::from(profile).join(".herdsman");
    if !dir.is_dir() {
        return;
    }

    for name in ["config.json", "settings.json", "status.json"] {
        let path = dir.join(name);
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        if let Some(ep) = json.get("endpoint").and_then(|v| v.as_str()) {
            push_api_base_candidate(candidates, ep);
        }
        if let Some(ep) = json.get("openai_endpoint").and_then(|v| v.as_str()) {
            push_api_base_candidate(candidates, ep);
        }
        if let Some(port) = json.get("port").and_then(|v| v.as_u64()) {
            push_api_base_candidate(candidates, &format!("http://127.0.0.1:{port}"));
        }
    }
}

#[cfg(not(windows))]
fn read_herdsman_config_candidates(candidates: &mut Vec<String>) {
    use std::fs;

    let Some(home) = dirs::home_dir() else {
        return;
    };
    let dir = home.join(".herdsman");
    if !dir.is_dir() {
        return;
    }

    for name in ["config.json", "settings.json", "status.json"] {
        let path = dir.join(name);
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        if let Some(ep) = json.get("endpoint").and_then(|v| v.as_str()) {
            push_api_base_candidate(candidates, ep);
        }
        if let Some(ep) = json.get("openai_endpoint").and_then(|v| v.as_str()) {
            push_api_base_candidate(candidates, ep);
        }
        if let Some(port) = json.get("port").and_then(|v| v.as_u64()) {
            push_api_base_candidate(candidates, &format!("http://127.0.0.1:{port}"));
        }
    }
}

fn collect_http_probe_candidates() -> Vec<String> {
    let mut candidates = Vec::new();

    if let Ok(guard) = runtime_state().lock() {
        if let Some(ref ep) = guard.endpoint {
            push_api_base_candidate(&mut candidates, ep);
        }
        if let Some(ref ep) = guard.openai_endpoint {
            push_api_base_candidate(&mut candidates, ep);
        }
    }

    read_herdsman_config_candidates(&mut candidates);

    for port in DEFAULT_HTTP_PORTS {
        push_api_base_candidate(&mut candidates, &format!("http://127.0.0.1:{port}"));
    }

    candidates
}

struct HttpProbeResult {
    base: String,
    openai_endpoint: String,
    models: Vec<HerdsmanModelInfo>,
}

fn probe_http_status() -> Option<HttpProbeResult> {
    let candidates = collect_http_probe_candidates();
    for base in candidates {
        let openai_endpoint = derive_openai_endpoint(&base);
        if let Some(models) = try_fetch_models(&base, &openai_endpoint) {
            herdsman_info(format!(
                "http probe ok: base={base} openai_endpoint={openai_endpoint}"
            ));
            return Some(HttpProbeResult {
                base,
                openai_endpoint,
                models,
            });
        }
    }
    None
}

fn http_still_reachable(base: &str, openai_endpoint: &str) -> bool {
    try_fetch_models(base, openai_endpoint).is_some()
}

fn apply_connected_state(
    app: &AppHandle,
    endpoint: String,
    openai_endpoint: String,
    models: Vec<HerdsmanModelInfo>,
    via: &str,
) {
    herdsman_info(format!("connected via {via}"));
    log_running_models(&models);
    emit_connected(app, true);
    emit_models(app, &models);
    update_runtime_state(|state| {
        state.connected = true;
        state.endpoint = Some(endpoint);
        state.openai_endpoint = Some(openai_endpoint);
        state.models = models;
    });
}

fn clear_connected_state(app: &AppHandle) {
    let was_connected = runtime_state()
        .lock()
        .map(|state| state.connected)
        .unwrap_or(false);
    if !was_connected {
        return;
    }
    herdsman_info("disconnected from herdsman");
    emit_connected(app, false);
    emit_models(app, &[]);
    update_runtime_state(|state| {
        state.connected = false;
        state.endpoint = None;
        state.openai_endpoint = None;
        state.models.clear();
    });
}

fn probe_and_update(app: &AppHandle) -> HerdsmanStatusSnapshot {
    #[cfg(windows)]
    if let Some(status) = read_status_from_pipe() {
        let client_endpoint = normalize_client_http_url(&status.endpoint);
        let client_openai_endpoint = normalize_client_http_url(&status.openai_endpoint);
        let models = fetch_models_from_status(&status);
        apply_connected_state(
            app,
            client_endpoint,
            client_openai_endpoint,
            models,
            "pipe",
        );
        return herdsman_get_status();
    }

    if let Some(result) = probe_http_status() {
        apply_connected_state(
            app,
            normalize_client_http_url(&result.base),
            result.openai_endpoint,
            result.models,
            "http",
        );
        return herdsman_get_status();
    }

    clear_connected_state(app);
    herdsman_get_status()
}

fn apply_launcher_discovery(app: &AppHandle, launcher: Option<String>) {
    let installed = launcher.is_some();
    let mut changed = false;

    update_runtime_state(|state| {
        if state.launcher_path != launcher || state.installed != installed {
            changed = true;
        }
        state.launcher_path = launcher.clone();
        state.installed = installed;
    });

    if changed {
        if installed {
            herdsman_info(format!(
                "installation state changed: installed at {}",
                launcher.as_deref().unwrap_or("?")
            ));
        } else {
            herdsman_info("installation state changed: not installed");
        }
        emit_install_detected(app, installed, launcher);
    }
}

fn run_discovery(app: &AppHandle) {
    herdsman_info("running installation discovery");
    let launcher = resolve_herdsman_launcher();
    apply_launcher_discovery(app, launcher);
}

fn discovery_loop(app: AppHandle) {
    run_discovery(&app);
    while SERVICE_RUNNING.load(Ordering::SeqCst) {
        sleep_interruptible(DISCOVER_INTERVAL);
        if SERVICE_RUNNING.load(Ordering::SeqCst) {
            run_discovery(&app);
        }
    }
}

fn poll_models_during_session(
    app: &AppHandle,
    endpoint: &str,
    openai_endpoint: &str,
    pipe_alive: &Arc<AtomicBool>,
) {
    while SERVICE_RUNNING.load(Ordering::SeqCst) && pipe_alive.load(Ordering::SeqCst) {
        sleep_interruptible(MODEL_POLL_INTERVAL);
        if !SERVICE_RUNNING.load(Ordering::SeqCst) || !pipe_alive.load(Ordering::SeqCst) {
            break;
        }
        let models = fetch_models(endpoint, openai_endpoint);
        let previous = runtime_state()
            .lock()
            .map(|state| models_signature(&state.models))
            .unwrap_or_default();
        let current = models_signature(&models);
        if current != previous {
            log_running_models(&models);
        }
        emit_models(app, &models);
        update_runtime_state(|state| {
            state.models = models;
        });
    }
}

#[cfg(windows)]
fn run_connection_session(app: &AppHandle) -> bool {
    let Some(handle) = connect_pipe() else {
        return false;
    };

    let mut endpoint = None;
    let mut openai_endpoint = None;
    let mut models = Vec::new();

    if let Some(response) = read_pipe_message(handle) {
        if let Ok(status) = serde_json::from_str::<HerdsmanStatus>(&response) {
            let client_endpoint = normalize_client_http_url(&status.endpoint);
            let client_openai_endpoint = normalize_client_http_url(&status.openai_endpoint);
            endpoint = Some(client_endpoint.clone());
            openai_endpoint = Some(client_openai_endpoint.clone());
            models = fetch_models_from_status(&status);
            herdsman_info(format!(
                "status received: endpoint={}, openai_endpoint={} (client: {}, {})",
                status.endpoint,
                status.openai_endpoint,
                client_endpoint,
                client_openai_endpoint,
            ));
        } else {
            herdsman_warn("failed to parse status pipe response");
        }
    } else {
        herdsman_warn("empty status pipe response");
    }

    apply_connected_state(
        app,
        endpoint.clone().unwrap_or_default(),
        openai_endpoint.clone().unwrap_or_default(),
        models,
        "pipe",
    );

    let pipe_alive = Arc::new(AtomicBool::new(true));
    if let Some(watch_handle) = duplicate_pipe_handle(handle) {
        let alive = Arc::clone(&pipe_alive);
        let watch = SendPipeHandle(watch_handle);
        thread::spawn(move || watch_pipe_disconnect(watch, alive));
    }

    if let (Some(endpoint), Some(openai_endpoint)) = (endpoint.as_deref(), openai_endpoint.as_deref()) {
        poll_models_during_session(app, endpoint, openai_endpoint, &pipe_alive);
    } else {
        while SERVICE_RUNNING.load(Ordering::SeqCst) && pipe_alive.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(200));
        }
    }

    close_pipe(handle);
    pipe_alive.store(false, Ordering::SeqCst);
    clear_connected_state(app);
    true
}

fn run_http_session(app: &AppHandle) {
    let Some(result) = probe_http_status() else {
        clear_connected_state(app);
        return;
    };

    let endpoint = normalize_client_http_url(&result.base);
    let openai_endpoint = result.openai_endpoint.clone();
    apply_connected_state(app, endpoint.clone(), openai_endpoint.clone(), result.models, "http");

    while SERVICE_RUNNING.load(Ordering::SeqCst) {
        sleep_interruptible(MODEL_POLL_INTERVAL);
        if !SERVICE_RUNNING.load(Ordering::SeqCst) {
            break;
        }
        if pipe_is_available() {
            herdsman_info("status pipe available; switching from http session");
            break;
        }
        if !http_still_reachable(&endpoint, &openai_endpoint) {
            herdsman_info("http session unreachable");
            break;
        }
        let models = fetch_models(&endpoint, &openai_endpoint);
        let previous = runtime_state()
            .lock()
            .map(|state| models_signature(&state.models))
            .unwrap_or_default();
        let current = models_signature(&models);
        if current != previous {
            log_running_models(&models);
        }
        emit_models(app, &models);
        update_runtime_state(|state| {
            state.models = models;
        });
    }

    clear_connected_state(app);
}

#[cfg(not(windows))]
fn run_connection_session(_app: &AppHandle) -> bool {
    false
}

fn service_loop(app: AppHandle) {
    while SERVICE_RUNNING.load(Ordering::SeqCst) {
        #[cfg(windows)]
        {
            if run_connection_session(&app) {
                // pipe session completed
            } else {
                run_http_session(&app);
            }
        }
        #[cfg(not(windows))]
        {
            run_http_session(&app);
        }

        if SERVICE_RUNNING.load(Ordering::SeqCst) {
            sleep_interruptible(RECONNECT_DELAY);
        }
    }

    clear_connected_state(&app);
}

pub fn start_herdsman_service(app: AppHandle) {
    if SERVICE_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    herdsman_info("herdsman service started");
    let discover_app = app.clone();
    thread::spawn(move || discovery_loop(discover_app));
    thread::spawn(move || service_loop(app));
}

pub fn stop_herdsman_service() {
    SERVICE_RUNNING.store(false, Ordering::SeqCst);
}

#[tauri::command]
pub fn herdsman_get_status() -> HerdsmanStatusSnapshot {
    runtime_state()
        .lock()
        .map(|state| snapshot_from_state(&state))
        .unwrap_or_default()
}

#[tauri::command]
pub fn herdsman_refresh_status(app: tauri::AppHandle) -> HerdsmanStatusSnapshot {
    run_discovery(&app);
    request_immediate_probe();
    probe_and_update(&app)
}

#[cfg(windows)]
fn is_valid_herdsman_exe(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

#[cfg(windows)]
fn resolve_from_callme_file() -> Option<String> {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let profile = env::var("USERPROFILE").ok()?;
    let callme = PathBuf::from(profile).join(".herdsman").join("callme");
    let content = fs::read_to_string(&callme).ok()?.trim().to_string();
    if content.is_empty() {
        return None;
    }
    let path = PathBuf::from(&content);
    if is_valid_herdsman_exe(&path) {
        Some(content)
    } else {
        None
    }
}

#[cfg(windows)]
fn is_herdsman_exe_file_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if !lower.ends_with(".exe") || lower == "uninstall.exe" {
        return false;
    }
    lower.starts_with("herdsman")
}

#[cfg(windows)]
fn desktop_dirs() -> Vec<std::path::PathBuf> {
    use std::env;
    use std::path::PathBuf;

    let Some(profile) = env::var("USERPROFILE").ok() else {
        return Vec::new();
    };
    let root = PathBuf::from(profile);
    let candidates = [
        root.join("Desktop"),
        root.join("桌面"),
        root.join("OneDrive").join("Desktop"),
        root.join("OneDrive").join("桌面"),
    ];
    candidates
        .into_iter()
        .filter(|path| path.is_dir())
        .collect()
}

#[cfg(windows)]
fn downloads_dirs() -> Vec<std::path::PathBuf> {
    use std::env;
    use std::path::PathBuf;

    let Some(profile) = env::var("USERPROFILE").ok() else {
        return Vec::new();
    };
    let root = PathBuf::from(profile);
    let candidates = [
        root.join("Downloads"),
        root.join("下载"),
    ];
    candidates
        .into_iter()
        .filter(|path| path.is_dir())
        .collect()
}

#[cfg(windows)]
fn newest_herdsman_exe_candidate(
    current: &mut Option<(std::time::SystemTime, String)>,
    path: std::path::PathBuf,
) {
    use std::fs;

    let mtime = match path.metadata().and_then(|meta| meta.modified()) {
        Ok(t) => t,
        Err(_) => return,
    };
    let path_str = path.to_string_lossy().into_owned();
    let replace = current
        .as_ref()
        .map(|(t, _)| mtime > *t)
        .unwrap_or(true);
    if replace {
        *current = Some((mtime, path_str));
    }
}

#[cfg(windows)]
fn resolve_from_shallow_dirs(dirs: &[std::path::PathBuf]) -> Option<String> {
    use std::fs;

    let mut best: Option<(std::time::SystemTime, String)> = None;

    for dir in dirs {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(name) => name,
                    None => continue,
                };
                if is_herdsman_exe_file_name(name) {
                    newest_herdsman_exe_candidate(&mut best, path);
                }
                continue;
            }
            if path.is_dir() {
                if let Some(found) = find_exe_in_dir(&path) {
                    newest_herdsman_exe_candidate(&mut best, found);
                }
            }
        }
    }

    best.map(|(_, path)| path)
}

#[cfg(windows)]
fn resolve_from_desktop() -> Option<String> {
    resolve_from_shallow_dirs(&desktop_dirs())
}

#[cfg(windows)]
fn resolve_from_downloads() -> Option<String> {
    resolve_from_shallow_dirs(&downloads_dirs())
}

#[cfg(windows)]
fn find_exe_in_dir(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::fs;

    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name()?.to_string_lossy();
        if is_herdsman_exe_file_name(&name) {
            return Some(path);
        }
    }
    None
}

#[cfg(windows)]
fn find_exe_in_install_dir(dir: &std::path::Path) -> Option<String> {
    if let Some(found) = find_exe_in_dir(dir) {
        return Some(found.to_string_lossy().into_owned());
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(found) = find_exe_in_dir(&path) {
            return Some(found.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(windows)]
fn resolve_known_install_paths() -> Option<String> {
    use std::env;
    use std::path::PathBuf;

    fn push_if_exists(candidates: &mut Vec<PathBuf>, path: Option<PathBuf>) {
        if let Some(p) = path {
            if p.exists() {
                candidates.push(p);
            }
        }
    }

    let local_app_data = env::var("LOCALAPPDATA").ok();
    let program_files = env::var("ProgramFiles").ok();
    let program_files_x86 = env::var("ProgramFiles(x86)").ok();
    let app_data = env::var("APPDATA").ok();

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(local) = &local_app_data {
        push_if_exists(
            &mut candidates,
            Some(PathBuf::from(local).join("Programs").join("Herdsman").join("Herdsman.exe")),
        );
        push_if_exists(
            &mut candidates,
            Some(
                PathBuf::from(local)
                    .join("Programs")
                    .join("starwave")
                    .join("Herdsman")
                    .join("herdsman.exe"),
            ),
        );
        push_if_exists(
            &mut candidates,
            Some(PathBuf::from(local).join("Herdsman").join("Herdsman.exe")),
        );
    }
    if let Some(pf) = &program_files {
        push_if_exists(
            &mut candidates,
            Some(PathBuf::from(pf).join("Herdsman").join("Herdsman.exe")),
        );
        push_if_exists(
            &mut candidates,
            Some(
                PathBuf::from(pf)
                    .join("starwave")
                    .join("Herdsman")
                    .join("herdsman.exe"),
            ),
        );
    }
    if let Some(pf) = &program_files_x86 {
        push_if_exists(
            &mut candidates,
            Some(PathBuf::from(pf).join("Herdsman").join("Herdsman.exe")),
        );
        push_if_exists(
            &mut candidates,
            Some(
                PathBuf::from(pf)
                    .join("starwave")
                    .join("Herdsman")
                    .join("herdsman.exe"),
            ),
        );
    }
    if let Some(ad) = &app_data {
        let start_menu = PathBuf::from(ad)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        for name in ["Herdsman.lnk", "牧马人.lnk", "Flowy Herdsman.lnk"] {
            push_if_exists(&mut candidates, Some(start_menu.join(name)));
        }
    }

    if let Some(path) = candidates.into_iter().next() {
        return Some(path.to_string_lossy().into_owned());
    }

    let scan_dirs: Vec<PathBuf> = [
        program_files.as_ref().map(|p| {
            PathBuf::from(p)
                .join("starwave")
                .join("Herdsman")
        }),
        program_files_x86.as_ref().map(|p| {
            PathBuf::from(p)
                .join("starwave")
                .join("Herdsman")
        }),
        local_app_data.as_ref().map(|p| {
            PathBuf::from(p)
                .join("Programs")
                .join("starwave")
                .join("Herdsman")
        }),
        local_app_data.as_ref().map(|p| PathBuf::from(p).join("Programs").join("Herdsman")),
        local_app_data.as_ref().map(|p| PathBuf::from(p).join("Herdsman")),
    ]
    .into_iter()
    .flatten()
    .collect();

    for dir in scan_dirs {
        if let Some(found) = find_exe_in_dir(&dir) {
            return Some(found.to_string_lossy().into_owned());
        }
    }

    for desktop in desktop_dirs() {
        if let Some(found) = find_exe_in_dir(&desktop) {
            return Some(found.to_string_lossy().into_owned());
        }
    }

    for downloads in downloads_dirs() {
        if let Some(found) = find_exe_in_dir(&downloads) {
            return Some(found.to_string_lossy().into_owned());
        }
    }

    None
}

#[cfg(windows)]
fn resolve_from_registry() -> Option<String> {
    use std::path::PathBuf;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    fn herdsman_display_name(name: &str) -> bool {
        let lower = name.to_lowercase();
        lower.contains("herdsman") || lower.contains("牧马人")
    }

    fn scan_uninstall(hive: winreg::HKEY, subkey: &str) -> Option<String> {
        let hk = RegKey::predef(hive);
        let uninstall = hk.open_subkey(subkey).ok()?;
        for subkey_name in uninstall.enum_keys().flatten() {
            let sub = match uninstall.open_subkey(&subkey_name) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let display_name: String = sub.get_value("DisplayName").unwrap_or_default();
            if !herdsman_display_name(&display_name) {
                continue;
            }
            let install_location: String = sub.get_value("InstallLocation").unwrap_or_default();
            let loc = install_location.trim();
            if loc.is_empty() {
                continue;
            }
            if let Some(exe) = find_exe_in_install_dir(PathBuf::from(loc).as_path()) {
                return Some(exe);
            }
        }
        None
    }

    let uninstall_paths = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    for (hive, path) in uninstall_paths {
        if let Some(exe) = scan_uninstall(hive, path) {
            return Some(exe);
        }
    }

    let app_paths = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths")
        .ok()?;
    for name in ["herdsman.exe", "Herdsman.exe"] {
        if let Ok(sub) = app_paths.open_subkey(name) {
            let path: String = sub.get_value("").or_else(|_| sub.get_value("Path")).unwrap_or_default();
            let path = path.trim().trim_matches('"').to_string();
            if !path.is_empty() {
                let candidate = if path.to_lowercase().ends_with(".exe") {
                    PathBuf::from(&path)
                } else {
                    PathBuf::from(&path).join("herdsman.exe")
                };
                if is_valid_herdsman_exe(&candidate) {
                    return Some(candidate.to_string_lossy().into_owned());
                }
            }
        }
    }

    None
}

#[cfg(windows)]
fn resolve_portable_exe_scan() -> Option<String> {
    use ignore::WalkBuilder;
    use std::env;
    use std::path::PathBuf;

    let mut dirs: Vec<PathBuf> = desktop_dirs();
    dirs.extend(downloads_dirs());
    if let Ok(local) = env::var("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local));
    }
    if let Ok(roaming) = env::var("APPDATA") {
        dirs.push(PathBuf::from(roaming));
    }

    let mut best: Option<(std::time::SystemTime, String)> = None;

    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let walker = WalkBuilder::new(&dir).max_depth(Some(3)).build();
        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !is_herdsman_exe_file_name(name) {
                continue;
            }
            let mtime = match path.metadata().and_then(|m| m.modified()) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let path_str = path.to_string_lossy().into_owned();
            let replace = best
                .as_ref()
                .map(|(t, _)| mtime > *t)
                .unwrap_or(true);
            if replace {
                best = Some((mtime, path_str));
            }
        }
    }

    best.map(|(_, path)| path)
}

#[cfg(windows)]
fn resolve_herdsman_launcher() -> Option<String> {
    let strategies: [(&str, fn() -> Option<String>); 6] = [
        ("callme file", resolve_from_callme_file),
        ("known install paths", resolve_known_install_paths),
        ("registry", resolve_from_registry),
        ("desktop", resolve_from_desktop),
        ("downloads", resolve_from_downloads),
        ("portable exe scan", resolve_portable_exe_scan),
    ];

    for (name, resolver) in strategies {
        match resolver() {
            Some(path) => {
                herdsman_info(format!("detected via {name}: {path}"));
                return Some(path);
            }
            None => herdsman_info(format!("{name}: not found")),
        }
    }

    herdsman_info("installation not detected by any strategy");
    None
}

#[cfg(not(windows))]
fn resolve_from_callme_file() -> Option<String> {
    use std::fs;
    use std::path::PathBuf;

    let home = dirs::home_dir()?;
    let callme = home.join(".herdsman").join("callme");
    let content = fs::read_to_string(&callme).ok()?.trim().to_string();
    if content.is_empty() {
        return None;
    }
    let path = PathBuf::from(&content);
    if is_valid_herdsman_launcher(&path) {
        Some(content)
    } else {
        None
    }
}

#[cfg(not(windows))]
fn resolve_known_install_paths() -> Option<String> {
    use std::path::PathBuf;

    fn push_if_exists(candidates: &mut Vec<PathBuf>, path: PathBuf) {
        if path.exists() {
            candidates.push(path);
        }
    }

    let mut candidates = Vec::new();
    if let Some(home) = dirs::home_dir() {
        push_if_exists(
            &mut candidates,
            home.join("Applications").join("Herdsman.app"),
        );
        push_if_exists(
            &mut candidates,
            home.join("Applications").join("牧马人.app"),
        );
    }
    for name in ["Herdsman.app", "牧马人.app", "Flowy Herdsman.app"] {
        push_if_exists(
            &mut candidates,
            PathBuf::from("/Applications").join(name),
        );
    }

    candidates
        .into_iter()
        .find_map(|path| launcher_path_from_bundle(&path))
}

#[cfg(not(windows))]
fn launcher_path_from_bundle(path: &std::path::Path) -> Option<String> {
    if path.extension().and_then(|ext| ext.to_str()) == Some("app") && path.is_dir() {
        return Some(path.to_string_lossy().into_owned());
    }
    if path.is_file() {
        return Some(path.to_string_lossy().into_owned());
    }
    None
}

#[cfg(not(windows))]
fn is_valid_herdsman_launcher(path: &std::path::Path) -> bool {
    if path.is_file() {
        return true;
    }
    path.extension().and_then(|ext| ext.to_str()) == Some("app") && path.is_dir()
}

#[cfg(not(windows))]
fn resolve_herdsman_launcher() -> Option<String> {
    let strategies: [(&str, fn() -> Option<String>); 2] = [
        ("callme file", resolve_from_callme_file),
        ("known install paths", resolve_known_install_paths),
    ];

    for (name, resolver) in strategies {
        match resolver() {
            Some(path) => {
                herdsman_info(format!("detected via {name}: {path}"));
                return Some(path);
            }
            None => herdsman_info(format!("{name}: not found")),
        }
    }

    herdsman_info("installation not detected by any strategy");
    None
}

fn cached_launcher_or_discover() -> Option<String> {
    if let Ok(guard) = runtime_state().lock() {
        if let Some(ref path) = guard.launcher_path {
            if is_launcher_present(std::path::Path::new(path)) {
                return Some(path.clone());
            }
        }
    }

    let discovered = resolve_herdsman_launcher();
    if discovered.is_some() {
        update_runtime_state(|state| {
            state.launcher_path = discovered.clone();
            state.installed = true;
        });
    }
    discovered
}

fn spawn_herdsman(launcher: &str) -> Result<(), String> {
    use std::path::Path;
    use std::process::Command;

    let path = Path::new(launcher);
    if !is_launcher_present(path) {
        herdsman_warn(format!("launcher path no longer exists: {launcher}"));
        update_runtime_state(|state| {
            state.launcher_path = None;
            state.installed = false;
        });
        return Err("Herdsman executable not found".into());
    }

    herdsman_info(format!("spawning Herdsman: {launcher}"));
    #[cfg(target_os = "macos")]
    if path.extension().and_then(|ext| ext.to_str()) == Some("app") {
        return Command::new("open")
            .arg("-a")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }

    let work_dir = path.parent().unwrap_or_else(|| Path::new("."));
    Command::new(path)
        .current_dir(work_dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn is_launcher_present(path: &std::path::Path) -> bool {
    if path.is_file() {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        if path.extension().and_then(|ext| ext.to_str()) == Some("app") && path.is_dir() {
            return true;
        }
    }
    false
}

#[tauri::command]
pub fn herdsman_start() -> Result<HerdsmanOpenResult, String> {
    let launcher = cached_launcher_or_discover()
        .ok_or_else(|| "Herdsman is not installed".to_string())?;
    spawn_herdsman(&launcher)?;
    request_immediate_probe();
    Ok(HerdsmanOpenResult {
        opened: "started".into(),
        target: launcher,
    })
}

#[tauri::command]
pub async fn herdsman_open_or_install(app: tauri::AppHandle) -> Result<HerdsmanOpenResult, String> {
    let connected = runtime_state()
        .lock()
        .map(|state| state.connected)
        .unwrap_or(false);

    if let Some(launcher) = cached_launcher_or_discover() {
        if connected {
            tauri_plugin_opener::open_path(launcher.clone(), None::<&str>)
                .map_err(|e| e.to_string())?;
            return Ok(HerdsmanOpenResult {
                opened: "app".into(),
                target: launcher,
            });
        }
        spawn_herdsman(&launcher)?;
        return Ok(HerdsmanOpenResult {
            opened: "started".into(),
            target: launcher,
        });
    }

    tauri_plugin_opener::open_url(DEFAULT_INSTALL_URL, None::<&str>)
        .map_err(|e| e.to_string())?;
    let _ = app;
    Ok(HerdsmanOpenResult {
        opened: "install-page".into(),
        target: DEFAULT_INSTALL_URL.into(),
    })
}

#[cfg(test)]
mod exe_name_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn accepts_herdsman_exe_variants() {
        assert!(is_herdsman_exe_file_name("herdsman.exe"));
        assert!(is_herdsman_exe_file_name("Herdsman.exe"));
        assert!(is_herdsman_exe_file_name("herdsman-v1.0.0-win.exe"));
        assert!(is_herdsman_exe_file_name("HerdsmanSetup.exe"));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_unrelated_exe_names() {
        assert!(!is_herdsman_exe_file_name("uninstall.exe"));
        assert!(!is_herdsman_exe_file_name("setup.exe"));
        assert!(!is_herdsman_exe_file_name("my-herdsman.exe"));
    }
}

#[cfg(test)]
mod map_tests {
    use super::*;

    #[test]
    fn map_models_reads_runtime_context_size() {
        let raw = vec![RawHerdsmanModel {
            id: None,
            name: "demo-model".into(),
            r#type: Some("text-generation".into()),
            status: Some("installed".into()),
            icon: None,
            runtime_info: Some(RuntimeInfo {
                context_size: Some(131_072),
                port: None,
                inference_engine: None,
                run_status: Some("running".into()),
            }),
        }];
        let models = map_models(raw, "http://127.0.0.1:8080/v1");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].context_window, Some(131_072));
    }
}

#[cfg(all(test, windows))]
mod status_tests {
    use super::*;

    #[test]
    fn herdsman_status_detects_running_instance() {
        let status = read_status_from_pipe().expect("Herdsman should be detectable when running");
        assert!(
            status.endpoint.contains("127.0.0.1"),
            "unexpected endpoint: {}",
            status.endpoint
        );
        assert!(!status.openai_endpoint.is_empty());
    }
}
