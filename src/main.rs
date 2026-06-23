use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use token_router::cli_settings::CliSettings;
use token_router::client;
use token_router::config::{self, ensure_initialized, load_from_path};
use token_router::daemon_ctl;
use token_router::env_cmd;
use token_router::gateway::{self, init_logging, AppConfig};
use token_router::setup_cmd;
use token_router::stats_cmd;
use tracing::info;

/// CLI for Flowy Router — gateway daemon and management commands.
/// Configuration: `~/.token-router/config.toml` (all platforms).
#[derive(Debug, Parser)]
#[command(name = "token-router", version, about)]
struct Cli {
    /// Override path to config.toml (default: ~/.token-router/config.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print resolved paths and configuration.
    Env {
        #[arg(long)]
        json: bool,
    },
    /// Show gateway routing and traffic statistics.
    Stats {
        /// Cumulative totals from `stats.json` (includes history across restarts).
        #[arg(long)]
        global: bool,
        #[arg(long)]
        json: bool,
        /// Human-readable output language: `en` (default) or `zh`.
        #[arg(long, default_value = "en", value_name = "LANG")]
        lang: String,
    },
    /// Initialize or update upstream model settings (cloud model=auto, edge empty by default).
    Setup {
        /// Apply via running gateway HTTP API instead of local config.toml.
        #[arg(long)]
        remote: bool,
        /// Skip interactive prompts (initialize defaults only).
        #[arg(long)]
        non_interactive: bool,
        #[arg(long)]
        json: bool,
        /// Reset to defaults (cloud model auto, edge cleared).
        #[arg(long)]
        reset: bool,
        #[arg(long)]
        cloud_url: Option<String>,
        #[arg(long)]
        cloud_key: Option<String>,
        #[arg(long)]
        cloud_model: Option<String>,
        #[arg(long)]
        edge_url: Option<String>,
        #[arg(long)]
        edge_key: Option<String>,
        #[arg(long)]
        edge_model: Option<String>,
        #[arg(long)]
        clear_edge: bool,
    },
    /// Manage the gateway daemon: start, stop, status, restart.
    #[command(subcommand)]
    Gateway(GatewayCommands),
    /// Hidden entry for the gateway daemon (re-invoked by `gateway start`).
    #[command(hide = true, name = "__serve")]
    Serve,
    /// Hidden helper: wait for `pid` to exit, then spawn `__serve` (used by POST /v1/admin/restart).
    #[command(hide = true, name = "__restart-wait")]
    RestartWait {
        /// PID of the gateway process that is shutting down.
        pid: u32,
    },
}

#[derive(Debug, Subcommand)]
enum GatewayCommands {
    Start {
        #[arg(long, default_value_t = 30)]
        wait: u64,
    },
    Stop {
        #[arg(short, long)]
        force: bool,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
    Restart {
        #[arg(long, default_value_t = 30)]
        wait: u64,
    },
}

fn ensure_settings(config_override: &Option<PathBuf>) -> Result<(CliSettings, bool)> {
    let (path, created) = ensure_initialized(config_override.as_deref())?;
    let (file, config_path) = load_from_path(&path)?;
    Ok((CliSettings { file, config_path }, created))
}

fn load_settings(config_override: &Option<PathBuf>) -> Result<CliSettings> {
    let path = match config_override {
        Some(p) => p.clone(),
        None => config::config_file()?,
    };
    let (file, config_path) = load_from_path(&path)?;
    Ok(CliSettings { file, config_path })
}

fn make_client(settings: &CliSettings) -> client::GatewayClient {
    client::GatewayClient::new(
        settings.gateway_url(),
        settings.api_key(),
        settings.admin_token(),
    )
}

fn print_init_message(created: bool, path: &std::path::Path) {
    if created {
        println!(
            "Created config at {} — edit upstream sections, then restart if needed.",
            path.display()
        );
    }
}

async fn run_serve(config_override: Option<PathBuf>) -> Result<()> {
    let app_config = AppConfig::load_from(config_override.as_deref())?;

    let log_path = init_logging(&app_config.data_dir, false)?;
    info!(
        config = %app_config.config_path.display(),
        app_dir = %app_config.data_dir.display(),
        log_file = %log_path.display(),
        "using config file"
    );

    gateway::daemon::assert_not_running(&app_config)?;
    info!(pid_file = %app_config.pid_file.display(), "starting gateway daemon");
    gateway::run(app_config, true).await
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_override = cli.config.clone();

    match cli.command {
        Commands::Serve => run_serve(config_override).await,
        Commands::RestartWait { pid } => daemon_ctl::run_restart_wait(pid, config_override.as_deref()),
        Commands::Env { json } => env_cmd::print_env(&config_override, json),
        Commands::Stats { global, json, lang } => {
            stats_cmd::print_stats(&config_override, global, json, &lang).await
        }
        Commands::Setup {
            remote,
            non_interactive,
            json,
            reset,
            cloud_url,
            cloud_key,
            cloud_model,
            edge_url,
            edge_key,
            edge_model,
            clear_edge,
        } => {
            let patch = setup_cmd::patch_from_cli(
                edge_url,
                edge_key,
                edge_model,
                cloud_url,
                cloud_key,
                cloud_model,
                clear_edge,
            );
            setup_cmd::run_setup(
                &config_override,
                remote,
                json,
                non_interactive,
                patch,
                reset,
            )
            .await
        }
        Commands::Gateway(cmd) => match cmd {
            GatewayCommands::Start { wait } => {
                let (settings, created) = ensure_settings(&config_override)?;
                print_init_message(created, &settings.config_path);
                let gw = make_client(&settings);
                daemon_ctl::start_daemon(&gw, &settings, wait).await
            }
            GatewayCommands::Stop { force } => {
                let settings = load_settings(&config_override)?;
                let gw = make_client(&settings);
                daemon_ctl::stop_daemon(&gw, force).await
            }
            GatewayCommands::Status { json } => {
                let settings = load_settings(&config_override)?;
                let gw = make_client(&settings);
                daemon_ctl::status_daemon(&gw, json).await
            }
            GatewayCommands::Restart { wait } => {
                let (settings, created) = ensure_settings(&config_override)?;
                print_init_message(created, &settings.config_path);
                let gw = make_client(&settings);
                daemon_ctl::restart_daemon(&gw, &settings, wait).await
            }
        },
    }
}
