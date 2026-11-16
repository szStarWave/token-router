//! `tr-cli` 入门命令：P0a 只有 `replay` 与 `check-config`。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "tr-cli", version, about = "Token Router v2 运维命令")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 回放一个 DecisionSnapshot，打印 Plan（不写库、不调判官）
    Replay {
        #[arg(long)]
        snapshot: PathBuf,
        /// 只打印指纹（用于两次回放对比）
        #[arg(long)]
        digest: bool,
        /// 紧凑 JSON 输出
        #[arg(long)]
        compact: bool,
    },
    /// 校验配置文件并打印目录摘要
    CheckConfig {
        #[arg(long)]
        config: PathBuf,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Replay {
            snapshot,
            digest,
            compact,
        } => {
            if digest {
                let text = std::fs::read_to_string(&snapshot)?;
                let snapshot: tr_contracts::DecisionSnapshot = serde_json::from_str(&text)?;
                println!("{}", tr_cli::replay_digest(&snapshot)?);
                return Ok(());
            }
            let plan = tr_cli::replay_file(&snapshot)?;
            if compact {
                println!("{}", serde_json::to_string(&plan)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
            Ok(())
        }
        Command::CheckConfig { config } => {
            let catalog = tr_agent::config::load(&config)?;
            println!(
                "ok: {} candidates, {} virtual models, default = {}, config_digest = {}",
                catalog.candidates.len(),
                catalog.virtual_models.len(),
                catalog.default_virtual_model,
                catalog.config_digest
            );
            Ok(())
        }
    }
}
