use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Application data directory name under user home (e.g. `~/{APP_DIR_NAME}/`).
#[cfg(feature = "desktop")]
pub const APP_DIR_NAME: &str = ".token-router-desktop";
#[cfg(not(feature = "desktop"))]
pub const APP_DIR_NAME: &str = ".token-router";

/// Process-wide app home for the running gateway (`--home`).
/// Agent one-click configure must read auth keys / listen addr from this home,
/// not the default `~/.token-router`.
static RUNTIME_APP_HOME: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Remember the active gateway data dir (set when the daemon loads config).
pub fn set_runtime_app_home(home: PathBuf) {
    if let Ok(mut guard) = RUNTIME_APP_HOME.write() {
        *guard = Some(home);
    }
}

/// Active gateway data dir, if the process has loaded a config.
pub fn runtime_app_home() -> Option<PathBuf> {
    RUNTIME_APP_HOME.read().ok().and_then(|g| g.clone())
}

/// User home directory (cross-platform).
///
/// - Linux / macOS: `$HOME`
/// - Windows: `%USERPROFILE%` / `dirs::home_dir()`
pub fn user_home() -> anyhow::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve user home directory"))
}

/// Default application root: `{user_home}/{APP_DIR_NAME}` (ignores runtime home).
pub fn app_dir() -> anyhow::Result<PathBuf> {
    Ok(user_home()?.join(APP_DIR_NAME))
}

/// Resolve application home: explicit `home`, else running gateway home, else `app_dir()`.
pub fn resolve_app_dir(home: Option<&Path>) -> anyhow::Result<PathBuf> {
    match home {
        Some(h) => Ok(h.to_path_buf()),
        None => {
            if let Some(h) = runtime_app_home() {
                return Ok(h);
            }
            app_dir()
        }
    }
}

pub fn resolve_config_file(home: Option<&Path>) -> anyhow::Result<PathBuf> {
    Ok(resolve_app_dir(home)?.join("config.toml"))
}

pub fn config_file() -> anyhow::Result<PathBuf> {
    resolve_config_file(None)
}

pub fn pid_file_at(app_home: &Path) -> PathBuf {
    app_home.join("gateway.pid")
}

pub fn pid_file() -> anyhow::Result<PathBuf> {
    Ok(pid_file_at(&app_dir()?))
}

pub fn sessions_dir_at(app_home: &Path) -> PathBuf {
    app_home.join("sessions")
}

pub fn sessions_dir() -> anyhow::Result<PathBuf> {
    Ok(sessions_dir_at(&app_dir()?))
}

pub fn logs_dir_at(app_home: &Path) -> PathBuf {
    app_home.join("logs")
}

pub fn logs_dir() -> anyhow::Result<PathBuf> {
    Ok(logs_dir_at(&app_dir()?))
}

pub fn gateway_log_file_at(app_home: &Path) -> PathBuf {
    logs_dir_at(app_home).join("gateway.log")
}

pub fn gateway_log_file() -> anyhow::Result<PathBuf> {
    Ok(gateway_log_file_at(&app_dir()?))
}

pub fn stats_file_at(app_home: &Path) -> PathBuf {
    app_home.join("stats.json")
}

pub fn stats_file() -> anyhow::Result<PathBuf> {
    Ok(stats_file_at(&app_dir()?))
}

pub fn stats_db_at(app_home: &Path) -> PathBuf {
    app_home.join("stats.db")
}

pub fn stats_db() -> anyhow::Result<PathBuf> {
    Ok(stats_db_at(&app_dir()?))
}

pub fn wordfreq_db_at(app_home: &Path) -> PathBuf {
    app_home.join("wordfreq.db")
}

pub fn wordfreq_db() -> anyhow::Result<PathBuf> {
    Ok(wordfreq_db_at(&app_dir()?))
}

/// `{app_home}/callme` — absolute path to the executable that can start Token Router.
pub fn callme_file_at(app_home: &Path) -> PathBuf {
    app_home.join("callme")
}

pub fn callme_file() -> anyhow::Result<PathBuf> {
    Ok(callme_file_at(&app_dir()?))
}

/// Write `{app_home}/callme` with the current executable path (best-effort).
fn ensure_callme_at(app_home: &Path) {
    let Ok(path) = std::env::current_exe() else {
        return;
    };
    if !path.is_file() {
        return;
    }
    let callme = callme_file_at(app_home);
    let content = format!("{}\n", path.display());
    let _ = std::fs::write(callme, content);
}

pub fn ensure_app_dirs(home: Option<&Path>) -> anyhow::Result<PathBuf> {
    let root = resolve_app_dir(home)?;
    std::fs::create_dir_all(&root)?;
    std::fs::create_dir_all(sessions_dir_at(&root))?;
    std::fs::create_dir_all(logs_dir_at(&root))?;
    ensure_callme_at(&root);
    Ok(root)
}

pub fn display_home() -> String {
    user_home()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~".to_string())
}

pub fn display_app_dir(home: Option<&Path>) -> String {
    resolve_app_dir(home)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| format!("{}/{}", display_home(), APP_DIR_NAME))
}

pub fn is_under_app_dir(path: &Path, home: Option<&Path>) -> bool {
    resolve_app_dir(home)
        .ok()
        .is_some_and(|root| path.starts_with(&root))
}
