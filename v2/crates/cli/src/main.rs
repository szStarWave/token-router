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
    /// shadow 记录聚合（一致率/延迟）
    ShadowReport {
        #[arg(long)]
        db: PathBuf,
        /// JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 判官自检（按配置构造运行时，跑一次固定问题）
    JudgeTest {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        json: bool,
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
        Command::ShadowReport { db, json } => {
            let db = tr_agent::store::Db::open(&db)?;
            let summary = tr_agent::store::judge::shadow_summary(&db)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "total": summary.total,
                        "agreed": summary.agreed,
                        "disagreed": summary.disagreed,
                        "unknown": summary.unknown,
                        "agreement_rate": summary.agreement_rate(),
                        "avg_latency_ms": summary.avg_latency_ms,
                    }))?
                );
            } else {
                println!(
                    "shadow: total={} agreed={} disagreed={} unknown={} rate={:?} avg_latency_ms={:?}",
                    summary.total,
                    summary.agreed,
                    summary.disagreed,
                    summary.unknown,
                    summary.agreement_rate(),
                    summary.avg_latency_ms
                );
            }
            Ok(())
        }
        Command::JudgeTest { config, json } => {
            let catalog = tr_agent::config::load(&config)?;
            let runtime = tr_agent::judge::JudgeRuntime::new(catalog.judge.clone())
                .map_err(|error| format!("judge runtime: {error}"))?;
            let runtime_tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime_tokio.block_on(runtime.self_test());
            if json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!(
                    "judge test: {:?} ({}ms){}{}",
                    result.outcome,
                    result.latency_ms,
                    result
                        .failure
                        .as_ref()
                        .map(|failure| format!(" code={:?}", failure.code))
                        .unwrap_or_default(),
                    result
                        .failure
                        .as_ref()
                        .and_then(|failure| failure.message.as_deref())
                        .map(|message| format!(" message={message}"))
                        .unwrap_or_default()
                );
            }
            if result.outcome != tr_contracts::TestOutcome::Passed {
                return Err("judge test failed".into());
            }
            Ok(())
        }
    }
}
