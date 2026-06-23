use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio_util::sync::CancellationToken;

use crate::gateway::config::AppConfig;
use crate::gateway::{init_logging, server};

struct EmbeddedGateway {
    cancel: CancellationToken,
    thread: JoinHandle<Result<()>>,
    gateway_url: String,
}

static EMBEDDED: Mutex<Option<EmbeddedGateway>> = Mutex::new(None);

/// Start the gateway inside the current process (for Electron / FFI embedding).
pub fn start(config_path: Option<&Path>) -> Result<String> {
    let mut guard = EMBEDDED
        .lock()
        .map_err(|_| anyhow::anyhow!("embedded gateway lock poisoned"))?;
    if guard.is_some() {
        bail!("gateway already running in this process");
    }

    let app_config = AppConfig::load_from(config_path)?;
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
                let log_path = init_logging(&app_config.data_dir, false)?;
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
                thread,
                gateway_url: gateway_url.clone(),
            });
            Ok(gateway_url)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            cancel.cancel();
            let _ = thread.join();
            bail!("embedded gateway did not become ready within 30s");
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let join_result = thread.join().unwrap_or_else(|_| Ok(()));
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
    embedded
        .thread
        .join()
        .map_err(|_| anyhow::anyhow!("embedded gateway thread panicked"))??;
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
