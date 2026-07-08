use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use crate::cli_settings::CliSettings;
use crate::config::pid_file_at;
use crate::gateway::config::AppConfig;
use crate::gateway::daemon;
use crate::gateway::{init_logging, LogRotateConfig};
use serde::Serialize;
use tracing::{info, warn};

use crate::client::GatewayClient;

#[derive(Debug, Serialize)]
struct StoppedStatus {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct UnknownStatus {
    status: &'static str,
    pid: u32,
    note: &'static str,
}

pub fn read_pid(app_home: &Path) -> Option<u32> {
    let path = pid_file_at(app_home);
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn resolve_gateway_bin() -> Result<std::path::PathBuf> {
    std::env::current_exe().context("current_exe")
}

fn append_home_port(cmd: &mut Command, settings: &CliSettings) {
    cmd.arg("--home").arg(&settings.app_home);
    if let Some(port) = settings.port {
        cmd.arg("--port").arg(port.to_string());
    }
}

pub async fn start_daemon(client: &GatewayClient, settings: &CliSettings, wait_secs: u64) -> Result<()> {
    if client.health().await.is_ok() {
        if let Ok(s) = client.status().await {
            println!(
                "gateway already running (pid {}, listen {})",
                s.pid, s.listen
            );
            return Ok(());
        }
        bail!("gateway already reachable at {}", client.base_url());
    }

    if let Some(pid) = read_pid(&settings.app_home) {
        if is_pid_alive(pid) {
            bail!("gateway already running (pid {pid})");
        }
        cleanup_stale_pid(&settings.app_home)?;
    }

    let bin = resolve_gateway_bin()?;
    let mut cmd = Command::new(&bin);
    append_home_port(&mut cmd, settings);
    cmd.arg("__serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))?;

    wait_until_healthy(client, wait_secs).await?;
    if let Ok(s) = client.status().await {
        println!(
            "gateway started (pid {}, listen {}, profile {})",
            s.pid, s.listen, s.default_profile
        );
    } else {
        println!("gateway started at {}", client.base_url());
    }
    Ok(())
}

pub async fn stop_daemon(client: &GatewayClient, settings: &CliSettings, force: bool) -> Result<()> {
    let pid = read_pid(&settings.app_home);
    let http_up = client.health().await.is_ok();

    if !http_up && pid.is_none() {
        println!("gateway is not running");
        return Ok(());
    }

    if http_up {
        let _ = client.shutdown().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        if client.health().await.is_err() {
            cleanup_stale_pid(&settings.app_home)?;
            println!("gateway stopped");
            return Ok(());
        }
    }

    if let Some(pid) = pid {
        if is_pid_alive(pid) {
            signal_stop(pid, force)?;
            for _ in 0..10 {
                if !is_pid_alive(pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        cleanup_stale_pid(&settings.app_home)?;
    }

    if client.health().await.is_ok() {
        bail!("gateway still reachable at {}", client.base_url());
    }

    println!("gateway stopped");
    Ok(())
}

pub async fn status_daemon(client: &GatewayClient, settings: &CliSettings, json: bool) -> Result<()> {
    match client.status().await {
        Ok(s) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                print_human_status(&s);
            }
            Ok(())
        }
        Err(_) => {
            if let Some(pid) = read_pid(&settings.app_home) {
                if is_pid_alive(pid) {
                    let u = UnknownStatus {
                        status: "unknown",
                        pid,
                        note: "process alive but HTTP unreachable",
                    };
                    if json {
                        println!("{}", serde_json::to_string_pretty(&u)?);
                    } else {
                        println!("Token Router");
                        println!("  Status:  unknown (pid {pid}, HTTP down)");
                        println!("  URL:     {}", client.base_url());
                    }
                    return Ok(());
                }
                cleanup_stale_pid(&settings.app_home)?;
            }
            let s = StoppedStatus { status: "stopped" };
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                println!("Token Router");
                println!("  Status:  stopped");
                println!("  URL:     {}", client.base_url());
            }
            Ok(())
        }
    }
}

pub async fn restart_daemon(
    client: &GatewayClient,
    settings: &CliSettings,
    wait_secs: u64,
) -> Result<()> {
    if client.health().await.is_ok() {
        client.restart().await?;
        wait_until_healthy(client, wait_secs).await?;
        if let Ok(s) = client.status().await {
            println!(
                "gateway restarted (pid {}, listen {}, profile {})",
                s.pid, s.listen, s.default_profile
            );
        } else {
            println!("gateway restarted at {}", client.base_url());
        }
        return Ok(());
    }

    cleanup_stale_pid(&settings.app_home)?;
    start_daemon(client, settings, wait_secs).await
}

/// Spawn a detached helper that waits for `old_pid` to exit, then starts `__serve`.
pub fn schedule_daemon_restart(app_home: &Path, old_pid: u32) -> Result<()> {
    let bin = resolve_gateway_bin()?;
    let mut cmd = Command::new(&bin);
    cmd.arg("--home")
        .arg(app_home)
        .arg("__restart-wait")
        .arg(old_pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach_child_process(&mut cmd);
    cmd.spawn()
        .with_context(|| format!("spawn restart helper {}", bin.display()))?;
    Ok(())
}

/// Wait until the gateway pid exits, then re-exec `__serve` (separate process from the dying gateway).
pub fn run_restart_wait(
    old_pid: u32,
    home: Option<&Path>,
    port: Option<u16>,
) -> Result<()> {
    let app_config = AppConfig::load_for_home(home, port)?;
    let log_path = init_logging(
        &app_config.data_dir,
        false,
        LogRotateConfig {
            max_size_mb: app_config.log_max_size_mb,
            max_files: app_config.log_max_files,
        },
    )?;
    info!(
        old_pid,
        config = %app_config.config_path.display(),
        log_file = %log_path.display(),
        "restart-wait: waiting for gateway to exit"
    );

    let deadline = Instant::now() + Duration::from_secs(60);
    while daemon::is_process_alive(old_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    if daemon::is_process_alive(old_pid) {
        bail!("timed out waiting for gateway pid {old_pid} to exit");
    }

    if daemon::read_pid(&app_config) == Some(old_pid) {
        daemon::remove_pid_file(&app_config);
    }
    std::thread::sleep(Duration::from_millis(400));

    let bin = resolve_gateway_bin()?;
    for attempt in 1..=20 {
        if daemon::is_running(&app_config) {
            info!("gateway already running after restart-wait");
            return Ok(());
        }
        let mut child_cmd = Command::new(&bin);
        child_cmd
            .arg("--home")
            .arg(&app_config.data_dir)
            .arg("__serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach_child_process(&mut child_cmd);
        match child_cmd.spawn() {
            Ok(mut child) => {
                std::thread::sleep(Duration::from_millis(800));
                match child.try_wait() {
                    Ok(None) => {
                        info!(
                            pid = child.id(),
                            attempt,
                            "restart-wait: spawned gateway"
                        );
                        return Ok(());
                    }
                    Ok(Some(status)) => {
                        warn!(%status, attempt, "restart-wait: __serve exited immediately");
                    }
                    Err(e) => warn!(error = %e, attempt, "restart-wait: try_wait failed"),
                }
            }
            Err(e) => warn!(error = %e, attempt, "restart-wait: spawn __serve failed"),
        }
        std::thread::sleep(Duration::from_millis(400));
    }

    bail!("failed to start gateway after pid {old_pid} exited")
}

#[cfg(unix)]
fn detach_child_process(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_child_process(_cmd: &mut Command) {}

fn print_human_status(s: &crate::client::GatewayStatus) {
    println!("Token Router");
    println!("  Status:   {}", s.status);
    println!("  Version:  {}", s.version);
    println!("  PID:      {}", s.pid);
    println!("  Listen:   {}", s.listen);
    println!("  Uptime:   {}s", s.uptime_secs);
    println!(
        "  Edge:     {}",
        if s.edge_configured {
            "configured"
        } else {
            "not configured"
        }
    );
    println!(
        "  Cloud:    {}",
        if s.cloud_configured {
            "configured"
        } else {
            "not configured"
        }
    );
    println!("  Profile:  {}", s.default_profile);
    println!("  PID file: {}", s.pid_file);
    println!("  Data dir: {}", s.data_dir);
}

async fn wait_until_healthy(client: &GatewayClient, secs: u64) -> Result<()> {
    for i in 0..secs {
        if client.health().await.is_ok() {
            return Ok(());
        }
        if i + 1 < secs {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    bail!(
        "gateway did not become healthy within {secs}s at {}",
        client.base_url()
    );
}

fn cleanup_stale_pid(app_home: &Path) -> Result<()> {
    let path = pid_file_at(app_home);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        use std::process::Command;
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

fn signal_stop(pid: u32, force: bool) -> Result<()> {
    #[cfg(unix)]
    {
        let sig = if force {
            libc::SIGKILL
        } else {
            libc::SIGTERM
        };
        let rc = unsafe { libc::kill(pid as i32, sig) };
        if rc != 0 {
            bail!("failed to signal pid {pid}");
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = force;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status();
        Ok(())
    }
}
