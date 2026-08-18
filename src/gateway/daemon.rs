use std::fs;
use std::io::Write;
use std::path::Path;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::gateway::config::AppConfig;

pub fn ensure_data_dir(config: &AppConfig) -> Result<()> {
    fs::create_dir_all(&config.data_dir).with_context(|| {
        format!("create data dir {}", config.data_dir.display())
    })?;
    Ok(())
}

pub fn write_pid_file(config: &AppConfig) -> Result<()> {
    ensure_data_dir(config)?;
    let pid = process::id();
    let mut file = fs::File::create(&config.pid_file)
        .with_context(|| format!("create pid file {}", config.pid_file.display()))?;
    writeln!(file, "{pid}")?;
    Ok(())
}

pub fn remove_pid_file(config: &AppConfig) {
    let _ = fs::remove_file(&config.pid_file);
}

pub fn read_pid(config: &AppConfig) -> Option<u32> {
    let text = fs::read_to_string(&config.pid_file).ok()?;
    text.trim().parse().ok()
}

pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        // Avoid trusting stale gateway.pid: Windows previously stubbed this as always
        // true, which blocked restart after crash. Without a Win32 OpenProcess dep,
        // treat pid-file presence alone as inconclusive (false); a live gateway still
        // holds the listen socket so a second bind will fail if one is running.
        let _ = pid;
        false
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        true
    }
}

pub fn is_running(config: &AppConfig) -> bool {
    read_pid(config)
        .map(is_process_alive)
        .unwrap_or(false)
}

pub fn assert_not_running(config: &AppConfig) -> Result<()> {
    if is_running(config) {
        let pid = read_pid(config).unwrap_or(0);
        bail!(
            "gateway already running (pid {pid}, pid file {})",
            config.pid_file.display()
        );
    }
    Ok(())
}

pub fn started_at_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn pid_file_path(config: &AppConfig) -> &Path {
    &config.pid_file
}
