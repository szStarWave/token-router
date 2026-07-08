use std::path::PathBuf;

use anyhow::Result;
use crate::config::{
    ConfigFile, gateway_log_file_at, load_from_path, logs_dir_at, pid_file_at, resolve_app_dir,
    sessions_dir_at, stats_db_at, stats_file_at, user_home,
};
use serde::Serialize;

use crate::cli_settings::CliSettings;
use crate::daemon_ctl;

#[derive(Debug, Serialize)]
pub struct EnvReport {
    pub paths: EnvPaths,
    pub config: EnvConfig,
    pub runtime: EnvRuntime,
}

#[derive(Debug, Serialize)]
pub struct EnvPaths {
    pub user_home: String,
    pub app_dir: String,
    pub config_file: String,
    pub config_exists: bool,
    pub pid_file: String,
    pub sessions_dir: String,
    pub logs_dir: String,
    pub gateway_log: String,
    pub stats_file: String,
    pub stats_db: String,
    pub gateway_pid: Option<u32>,
    pub gateway_bin: String,
}

#[derive(Debug, Serialize)]
pub struct EnvConfig {
    pub gateway_listen: String,
    pub gateway_url: String,
    pub route: String,
    pub routing_mode: String,
    pub default_profile: String,
    pub ctx_edge_max_tokens: u32,
    pub gateway_api_key_set: bool,
    pub admin_token_set: bool,
    pub edge_upstream: Option<String>,
    pub cloud_upstream: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EnvRuntime {
    /// Process environment (not Flowy config). Logging only.
    pub rust_log: Option<String>,
}

pub fn print_env(home: &Option<PathBuf>, port: Option<u16>, json: bool) -> Result<()> {
    let report = build_report(home, port)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }
    Ok(())
}

fn build_report(home: &Option<PathBuf>, port: Option<u16>) -> Result<EnvReport> {
    let app_home = resolve_app_dir(home.as_deref())?;
    let config_path = app_home.join("config.toml");
    let config_exists = config_path.exists();
    let file = if config_exists {
        load_from_path(&config_path)?.0
    } else {
        ConfigFile::default()
    };
    let settings = CliSettings::from_parts(file.clone(), app_home.clone(), port);

    let gateway_bin = daemon_ctl::resolve_gateway_bin()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("(not found: {e})"));

    let edge_upstream = file
        .upstream
        .edge
        .as_ref()
        .map(|e| e.base_url.clone())
        .filter(|s| !s.is_empty());
    let cloud_upstream = file
        .upstream
        .cloud
        .as_ref()
        .map(|c| c.base_url.clone())
        .filter(|s| !s.is_empty());

    Ok(EnvReport {
        paths: EnvPaths {
            user_home: user_home()?.display().to_string(),
            app_dir: app_home.display().to_string(),
            config_file: config_path.display().to_string(),
            config_exists,
            pid_file: pid_file_at(&app_home).display().to_string(),
            sessions_dir: sessions_dir_at(&app_home).display().to_string(),
            logs_dir: logs_dir_at(&app_home).display().to_string(),
            gateway_log: gateway_log_file_at(&app_home).display().to_string(),
            stats_file: stats_file_at(&app_home).display().to_string(),
            stats_db: stats_db_at(&app_home).display().to_string(),
            gateway_pid: daemon_ctl::read_pid(&app_home),
            gateway_bin,
        },
        config: EnvConfig {
            gateway_listen: file.gateway.listen.clone(),
            gateway_url: settings.gateway_url(),
            route: file.gateway.route.clone(),
            routing_mode: file.gateway.routing_mode.clone(),
            default_profile: file.gateway.default_profile.clone(),
            ctx_edge_max_tokens: file.gateway.ctx_edge_max_tokens,
            gateway_api_key_set: file
                .gateway
                .api_key
                .as_ref()
                .is_some_and(|s| !s.is_empty()),
            admin_token_set: file
                .gateway
                .admin_token
                .as_ref()
                .is_some_and(|s| !s.is_empty()),
            edge_upstream,
            cloud_upstream,
        },
        runtime: EnvRuntime {
            rust_log: std::env::var("RUST_LOG").ok().filter(|s| !s.is_empty()),
        },
    })
}

fn print_human(r: &EnvReport) {
    println!("Paths");
    println!("  user_home:      {}", r.paths.user_home);
    println!("  app_dir:        {}", r.paths.app_dir);
    println!(
        "  config_file:    {} ({})",
        r.paths.config_file,
        if r.paths.config_exists {
            "exists"
        } else {
            "missing — run `token-router gateway start` to create"
        }
    );
    println!("  pid_file:       {}", r.paths.pid_file);
    println!("  sessions_dir:   {}", r.paths.sessions_dir);
    println!("  logs_dir:       {}", r.paths.logs_dir);
    println!("  gateway_log:    {}", r.paths.gateway_log);
    println!("  stats_file:     {}", r.paths.stats_file);
    println!("  stats_db:       {}", r.paths.stats_db);
    println!(
        "  gateway_pid:    {}",
        r.paths
            .gateway_pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "(not running)".to_string())
    );
    println!("  gateway_bin:    {}", r.paths.gateway_bin);

    println!();
    println!("Config (from config.toml or defaults if missing)");
    println!("  gateway_listen:       {}", r.config.gateway_listen);
    println!("  gateway_url:          {}", r.config.gateway_url);
    println!("  route:                {}", r.config.route);
    println!("  routing_mode:         {}", r.config.routing_mode);
    println!("  default_profile:      {}", r.config.default_profile);
    println!("  ctx_edge_max_tokens:  {}", r.config.ctx_edge_max_tokens);
    println!(
        "  gateway_api_key_set:  {}",
        r.config.gateway_api_key_set
    );
    println!(
        "  admin_token_set:      {}",
        r.config.admin_token_set
    );
    println!(
        "  edge_upstream:        {}",
        r.config
            .edge_upstream
            .as_deref()
            .unwrap_or("(not set)")
    );
    println!(
        "  cloud_upstream:       {}",
        r.config
            .cloud_upstream
            .as_deref()
            .unwrap_or("(not set)")
    );
    println!();
    println!("Runtime environment");
    println!(
        "  RUST_LOG:       {}",
        r.runtime
            .rust_log
            .as_deref()
            .unwrap_or("(not set)")
    );
}
