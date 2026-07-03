use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};
use token_router::embedded;
use token_router::gateway::AppConfig;

pub const PIPE_NAME: &str = r"\\.\pipe\Token-Router-status";
const APP_NAME: &str = "Token Router";
const BUF_SIZE: u32 = 4096;

#[derive(Clone, Debug, Default)]
struct PipeState {
    gateway_running: bool,
    host: String,
    port: u16,
    gateway_base: String,
}

#[derive(Serialize)]
struct ServerStatus<'a> {
    app_name: &'static str,
    running: bool,
    host: &'a str,
    port: u16,
    endpoint: &'a str,
    webui_url: &'a str,
    openai_endpoint: &'a str,
    chat_endpoint: &'a str,
    responses_endpoint: &'a str,
    anthropic_endpoint: &'a str,
    token: &'a str,
    timestamp: String,
}

struct EndpointUrls {
    endpoint: String,
    webui_url: String,
    openai_endpoint: String,
    chat_endpoint: String,
    responses_endpoint: String,
    anthropic_endpoint: String,
}

struct PipeService {
    server_running: Arc<AtomicBool>,
    state: Arc<Mutex<PipeState>>,
    thread: Option<JoinHandle<()>>,
}

static SERVICE: OnceLock<Mutex<Option<PipeService>>> = OnceLock::new();
#[cfg(windows)]
static ACTIVE_PIPE: AtomicIsize = AtomicIsize::new(0);

fn service_slot() -> &'static Mutex<Option<PipeService>> {
    SERVICE.get_or_init(|| Mutex::new(None))
}

pub fn start() {
    #[cfg(windows)]
    {
        let mut guard = service_slot().lock().expect("status pipe lock. poisoned");
        if guard.is_some() {
            return;
        }
        sync_gateway_state_inner();
        let server_running = Arc::new(AtomicBool::new(true));
        let state = Arc::new(Mutex::new(current_pipe_state()));
        let running_for_thread = Arc::clone(&server_running);
        let state_for_thread = Arc::clone(&state);
        let thread = thread::Builder::new()
            .name("token-router-status-pipe".into())
            .spawn(move || listen_loop(running_for_thread, state_for_thread))
            .expect("spawn status pipe thread");
        *guard = Some(PipeService {
            server_running,
            state,
            thread: Some(thread),
        });
    }
}

pub fn stop() {
    #[cfg(windows)]
    {
        let mut guard = service_slot().lock().expect("status pipe mutex poisoned");
        if let Some(mut service) = guard.take() {
            service.server_running.store(false, Ordering::SeqCst);
            interrupt_active_pipe();
            if let Some(thread) = service.thread.take() {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(thread.join());
                });
                let _ = rx.recv_timeout(std::time::Duration::from_secs(2));
            }
        }
    }
}

pub fn sync_gateway_state() {
    sync_gateway_state_inner();
    #[cfg(windows)]
    {
        if let Ok(guard) = service_slot().lock() {
            if let Some(service) = guard.as_ref() {
                if let Ok(mut state) = service.state.lock() {
                    *state = current_pipe_state();
                }
            }
        }
    }
}

fn sync_gateway_state_inner() {
    // Reserved for shared state refresh; pipe state is derived from embedded gateway on read.
    let _ = embedded::is_running();
}

fn current_pipe_state() -> PipeState {
    if embedded::is_running() {
        if let Some(base) = embedded::gateway_url() {
            let base = base.trim_end_matches('/').to_string();
            if let Some((host, port)) = parse_http_base(&base) {
                return PipeState {
                    gateway_running: true,
                    host,
                    port,
                    gateway_base: base,
                };
            }
        }
    }
    PipeState::default()
}

fn parse_http_base(url: &str) -> Option<(String, u16)> {
    let trimmed = url.trim().trim_end_matches('/');
    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))?;
    let (host, port) = without_scheme.rsplit_once(':')?;
    let port = port.parse().ok()?;
    Some((host.to_string(), port))
}

fn build_endpoint_urls(base: &str) -> EndpointUrls {
    let base = base.trim_end_matches('/');
    EndpointUrls {
        endpoint: base.to_string(),
        webui_url: format!("{base}/setup"),
        openai_endpoint: format!("{base}/v1"),
        chat_endpoint: format!("{base}/v1"),
        responses_endpoint: format!("{base}/v1/responses"),
        anthropic_endpoint: format!("{base}/anthropic"),
    }
}

fn empty_endpoint_urls() -> EndpointUrls {
    EndpointUrls {
        endpoint: String::new(),
        webui_url: String::new(),
        openai_endpoint: String::new(),
        chat_endpoint: String::new(),
        responses_endpoint: String::new(),
        anthropic_endpoint: String::new(),
    }
}

fn resolve_status_token() -> String {
    AppConfig::load()
        .ok()
        .and_then(|config| config.api_key)
        .unwrap_or_default()
}

fn rfc3339_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Good enough for status IPC; full RFC3339 is not required by clients.
    format!("{secs}")
}

pub fn build_status_response(state: &PipeState) -> String {
    let urls = if state.gateway_running && !state.gateway_base.is_empty() {
        build_endpoint_urls(&state.gateway_base)
    } else {
        empty_endpoint_urls()
    };

    let token = if state.gateway_running {
        resolve_status_token()
    } else {
        String::new()
    };

    let status = ServerStatus {
        app_name: APP_NAME,
        running: state.gateway_running,
        host: if state.gateway_running {
            state.host.as_str()
        } else {
            ""
        },
        port: if state.gateway_running {
            state.port
        } else {
            0
        },
        endpoint: &urls.endpoint,
        webui_url: &urls.webui_url,
        openai_endpoint: &urls.openai_endpoint,
        chat_endpoint: &urls.chat_endpoint,
        responses_endpoint: &urls.responses_endpoint,
        anthropic_endpoint: &urls.anthropic_endpoint,
        token: &token,
        timestamp: rfc3339_now(),
    };

    serde_json::to_string_pretty(&status).unwrap_or_else(|_| {
        r#"{"error":"failed to marshal status"}"#.to_string()
    })
}

#[cfg(windows)]
fn store_active_pipe(handle: windows::Win32::Foundation::HANDLE) {
    ACTIVE_PIPE.store(handle.0 as isize, Ordering::SeqCst);
}

#[cfg(windows)]
fn take_active_pipe() -> Option<windows::Win32::Foundation::HANDLE> {
    let raw = ACTIVE_PIPE.swap(0, Ordering::SeqCst);
    if raw == 0 {
        None
    } else {
        Some(windows::Win32::Foundation::HANDLE(
            raw as *mut core::ffi::c_void,
        ))
    }
}

#[cfg(windows)]
fn interrupt_active_pipe() {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Pipes::DisconnectNamedPipe;

    if let Some(handle) = take_active_pipe() {
        unsafe {
            let _ = DisconnectNamedPipe(handle);
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(windows)]
struct ActiveConnectionGuard;

#[cfg(windows)]
impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        interrupt_active_pipe();
    }
}

#[cfg(windows)]
fn listen_loop(server_running: Arc<AtomicBool>, state: Arc<Mutex<PipeState>>) {
    while server_running.load(Ordering::SeqCst) {
        if let Err(err) = handle_connection(&server_running, &state) {
            if !server_running.load(Ordering::SeqCst) {
                break;
            }
            eprintln!("status pipe connection error: {err}");
        }
    }
}

#[cfg(windows)]
fn handle_connection(
    server_running: &Arc<AtomicBool>,
    _state: &Arc<Mutex<PipeState>>,
) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, INVALID_HANDLE_VALUE};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::{
        FlushFileBuffers, ReadFile, WriteFile, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
        PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES,
    };

    let wide: Vec<u16> = OsStr::new(PIPE_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut security_descriptor = windows::Win32::Security::PSECURITY_DESCRIPTOR::default();
    let sddl: Vec<u16> = OsStr::new("D:(A;;GA;;;WD)")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut security_descriptor,
            None,
        )
        .map_err(|e| format!("security descriptor: {e}"))?;
    }

    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security_descriptor.0,
        bInheritHandle: false.into(),
    };

    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE,
            PIPE_UNLIMITED_INSTANCES,
            BUF_SIZE,
            BUF_SIZE,
            0,
            Some(&mut sa),
        )
    };

    unsafe {
        let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            security_descriptor.0 as _,
        )));
    }

    if handle == INVALID_HANDLE_VALUE {
        return Err("CreateNamedPipeW failed".to_string());
    }

    store_active_pipe(handle);
    let _connection_guard = ActiveConnectionGuard;

    unsafe {
        if let Err(e) = ConnectNamedPipe(handle, None) {
            let code = e.code().0 as u32;
            if code != windows::Win32::Foundation::ERROR_PIPE_CONNECTED.0 {
                let _ = DisconnectNamedPipe(handle);
                let _ = CloseHandle(handle);
                return Err(format!("ConnectNamedPipe: {e}"));
            }
        }
    }

    let mut buf = vec![0u8; BUF_SIZE as usize];
    loop {
        if !server_running.load(Ordering::SeqCst) {
            break;
        }

        let mut read = 0u32;
        let read_result = unsafe { ReadFile(handle, Some(&mut buf), Some(&mut read), None) };
        match read_result {
            Ok(()) => {}
            Err(e) => {
                let code = e.code().0 as u32;
                if code == windows::Win32::Foundation::ERROR_BROKEN_PIPE.0 {
                    break;
                }
                return Err(format!("ReadFile: {e}"));
            }
        }

        if read == 0 {
            continue;
        }

        let command = String::from_utf8_lossy(&buf[..read as usize])
            .trim()
            .to_string();

        let response = if command == "/status" {
            build_status_response(&current_pipe_state())
        } else if command == "/exit" {
            let body = r#"{"message":"connection closed"}"#;
            let mut written = 0u32;
            unsafe {
                let _ = WriteFile(handle, Some(body.as_bytes()), Some(&mut written), None);
                let _ = FlushFileBuffers(handle);
            }
            break;
        } else {
            r#"{"error":"unknown command"}"#.to_string()
        };

        let mut written = 0u32;
        unsafe {
            WriteFile(
                handle,
                Some(response.as_bytes()),
                Some(&mut written),
                None,
            )
            .map_err(|e| format!("WriteFile: {e}"))?;
            let _ = FlushFileBuffers(handle);
        }
    }

    Ok(())
}

#[cfg(not(windows))]
fn listen_loop(_server_running: Arc<AtomicBool>, _state: Arc<Mutex<PipeState>>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_status_response_running_urls() {
        let state = PipeState {
            gateway_running: true,
            host: "127.0.0.1".to_string(),
            port: 11080,
            gateway_base: "http://127.0.0.1:11080".to_string(),
        };
        let json = build_status_response(&state);
        assert!(json.contains(r#""endpoint": "http://127.0.0.1:11080""#));
        assert!(json.contains(r#""openai_endpoint": "http://127.0.0.1:11080/v1""#));
        assert!(json.contains(r#""chat_endpoint": "http://127.0.0.1:11080/v1""#));
        assert!(json.contains(
            r#""responses_endpoint": "http://127.0.0.1:11080/v1/responses""#
        ));
        assert!(json.contains(
            r#""anthropic_endpoint": "http://127.0.0.1:11080/anthropic""#
        ));
        assert!(json.contains(r#""webui_url": "http://127.0.0.1:11080/setup""#));
        assert!(json.contains(r#""token""#));
        assert!(json.contains(r#""running": true"#));
    }

    #[test]
    fn build_status_response_stopped_empty_urls() {
        let state = PipeState::default();
        let json = build_status_response(&state);
        assert!(json.contains(r#""running": false"#));
        assert!(json.contains(r#""endpoint": """#));
        assert!(json.contains(r#""chat_endpoint": """#));
        assert!(json.contains(r#""responses_endpoint": """#));
        assert!(json.contains(r#""anthropic_endpoint": """#));
        assert!(json.contains(r#""token": """#));
    }

    #[test]
    fn parse_http_base_extracts_host_and_port() {
        let (host, port) = parse_http_base("http://127.0.0.1:11080").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 11080);
    }

    #[test]
    fn build_endpoint_urls_from_wildcard_mapped_base() {
        let urls = build_endpoint_urls("http://127.0.0.1:11080");
        assert_eq!(urls.chat_endpoint, "http://127.0.0.1:11080/v1");
        assert_eq!(urls.openai_endpoint, urls.chat_endpoint);
        assert_eq!(urls.responses_endpoint, "http://127.0.0.1:11080/v1/responses");
        assert_eq!(urls.anthropic_endpoint, "http://127.0.0.1:11080/anthropic");
    }
}
