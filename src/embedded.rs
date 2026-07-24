use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio_util::sync::CancellationToken;

use crate::gateway::config::AppConfig;
use crate::gateway::{init_logging, server, LogRotateConfig};

struct EmbeddedGateway {
    cancel: CancellationToken,
    thread: Option<JoinHandle<Result<()>>>,
    gateway_url: String,
}

static EMBEDDED: Mutex<Option<EmbeddedGateway>> = Mutex::new(None);

/// Start the gateway inside the current process (for Electron / FFI embedding).
pub fn start(home: Option<&Path>, port: Option<u16>) -> Result<String> {
    let mut guard = EMBEDDED
        .lock()
        .map_err(|_| anyhow::anyhow!("embedded gateway lock poisoned"))?;
    if guard.is_some() {
        bail!("gateway already running in this process");
    }

    let app_config = AppConfig::load_for_home(home, port)?;
    let gateway_url = app_config.gateway_base_url();
    let cancel = CancellationToken::new();
    let cancel_for_run = cancel.clone();

    let (ready_tx, ready_rx) = mpsc::sync_channel(1);

    let thread = thread::Builder::new()
        .name("token-router-gateway".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("token-router-gateway-worker")
                .build()
                .context("create tokio runtime")?;

            rt.block_on(async move {
                let log_path = init_logging(
                    &app_config.data_dir,
                    false,
                    LogRotateConfig {
                        max_size_mb: app_config.log_max_size_mb,
                        max_files: app_config.log_max_files,
                    },
                )?;
                tracing::info!(
                    config = %app_config.config_path.display(),
                    app_dir = %app_config.data_dir.display(),
                    log_file = %log_path.display(),
                    "embedded gateway starting"
                );

                server::run_with_options(
                    app_config,
                    server::RunOptions::embedded(cancel_for_run, ready_tx),
                )
                .await
            })
        })
        .context("spawn embedded gateway thread")?;

    match ready_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(()) => {
            *guard = Some(EmbeddedGateway {
                cancel,
                thread: Some(thread),
                gateway_url: gateway_url.clone(),
            });
            Ok(gateway_url)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if probe_gateway_health(&gateway_url) {
                tracing::info!("embedded gateway already running at {gateway_url}, adopting");
                *guard = Some(EmbeddedGateway {
                    cancel,
                    thread: Some(thread),
                    gateway_url: gateway_url.clone(),
                });
                return Ok(gateway_url);
            }
            cancel.cancel();
            let _ = thread.join();
            bail!("embedded gateway did not become ready within 30s");
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let join_result = thread.join().unwrap_or_else(|_| Ok(()));
            if probe_gateway_health(&gateway_url) {
                tracing::info!(
                    "embedded gateway thread exited but {} is healthy, adopting",
                    gateway_url
                );
                *guard = Some(EmbeddedGateway {
                    cancel,
                    thread: None,
                    gateway_url: gateway_url.clone(),
                });
                return Ok(gateway_url);
            }
            if let Err(join_err) = join_result {
                return Err(join_err);
            }
            bail!("embedded gateway exited before becoming ready");
        }
    }
}

/// Stop the in-process gateway and wait for the worker thread to exit.
pub fn stop() -> Result<()> {
    let mut guard = EMBEDDED
        .lock()
        .map_err(|_| anyhow::anyhow!("embedded gateway lock poisoned"))?;
    let embedded = guard.take().context("gateway is not running in this process")?;

    embedded.cancel.cancel();
    if let Some(thread) = embedded.thread {
        thread
            .join()
            .map_err(|_| anyhow::anyhow!("embedded gateway thread panicked"))??;
    }
    Ok(())
}

/// Whether the in-process gateway is running.
pub fn is_running() -> bool {
    EMBEDDED
        .lock()
        .ok()
        .is_some_and(|guard| guard.is_some())
}

/// Base URL of the running embedded gateway (`http://host:port`).
pub fn gateway_url() -> Option<String> {
    EMBEDDED
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|g| g.gateway_url.clone()))
}

/// Check whether a gateway is already serving at `url` by requesting `/health`.
/// Uses a simple TCP connection to avoid introducing a runtime dependency.
fn probe_gateway_health(url: &str) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let health_url = format!("{}/health", url.trim_end_matches('/'));
    let addr = match health_url.strip_prefix("http://") {
        Some(rest) => match rest.split_once('/') {
            Some((host_port, _)) => host_port,
            None => rest,
        },
        None => return false,
    };
    let socket_addr: std::net::SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };

    let mut stream = match TcpStream::connect_timeout(&socket_addr, Duration::from_secs(3)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));

    let request = format!(
        "GET /health HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        addr
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };

    let response = String::from_utf8_lossy(&buf[..n]);
    response.contains(r#""status":"ok""#) || response.contains(r#""status": "ok""#)
}
