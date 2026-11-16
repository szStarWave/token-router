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
    /// 任务集评测（成功率/成本/时延）
    Eval {
        #[arg(long)]
        config: PathBuf,
        /// 任务集 JSON
        #[arg(long, default_value = "tests/eval/tasks.json")]
        tasks: PathBuf,
        #[arg(long)]
        json: bool,
        /// 指定账本库（默认临时库）
        #[arg(long)]
        db: Option<PathBuf>,
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
        Command::Eval {
            config,
            tasks,
            json,
            db,
        } => run_eval(config, tasks, json, db),
    }
}

#[derive(Debug, serde::Deserialize)]
struct EvalTask {
    id: String,
    prompt: String,
    expect: EvalExpect,
}

#[derive(Debug, serde::Deserialize)]
struct EvalExpect {
    contains: String,
}

fn run_eval(
    config: PathBuf,
    tasks_path: PathBuf,
    json: bool,
    db_path: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = tr_agent::config::load(&config)?;
    let tasks_text = std::fs::read_to_string(&tasks_path)?;
    let tasks: Vec<EvalTask> = serde_json::from_str::<serde_json::Value>(&tasks_text)?
        .get("tasks")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .ok_or("tasks file 缺少 tasks 数组")?;

    let db_path = db_path.unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "tr-eval-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0)
        ))
    });
    let db = tr_agent::store::Db::open(&db_path)?;
    let pipeline = tr_agent::pipeline::Pipeline::from_catalog(db, catalog);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let mut results = Vec::new();
    for task in &tasks {
        let request = tr_contracts::CanonicalRequest {
            messages: vec![tr_contracts::CanonicalMessage {
                role: tr_contracts::CanonicalRole::User,
                blocks: vec![tr_contracts::ContentBlock::Text {
                    text: task.prompt.clone(),
                    cache_control: None,
                }],
                ..tr_contracts::CanonicalMessage::default()
            }],
            ..tr_contracts::CanonicalRequest::default()
        };

        let outcome = runtime
            .block_on(pipeline.handle(request, tr_agent::pipeline::RequestContext::default()));
        match outcome {
            Ok(result) => {
                let text: String = result
                    .response
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        tr_contracts::ContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                let passed = text.contains(&task.expect.contains);
                results.push(serde_json::json!({
                    "id": task.id,
                    "passed": passed,
                    "served_model": result.headers.served_model,
                    "cost_micros": result.ledger.cost_equiv.as_ref().map(|cost| cost.micros),
                    "duration_ms": result.ledger.duration_ms,
                }));
            }
            Err(error) => {
                results.push(serde_json::json!({
                    "id": task.id,
                    "passed": false,
                    "error": error.to_string(),
                }));
            }
        }
    }

    let total = results.len();
    let passed = results
        .iter()
        .filter(|result| result["passed"] == serde_json::Value::Bool(true))
        .count();
    let cost: i64 = results
        .iter()
        .filter_map(|result| result["cost_micros"].as_i64())
        .sum();
    let duration: i64 = results
        .iter()
        .filter_map(|result| result["duration_ms"].as_i64())
        .sum();

    let summary = serde_json::json!({
        "tasks": total,
        "passed": passed,
        "success_rate": if total > 0 { Some(passed as f64 / total as f64) } else { None },
        "total_cost_micros": cost,
        "avg_duration_ms": if total > 0 { Some(duration as f64 / total as f64) } else { None },
        "results": results,
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "eval: {passed}/{total} passed, cost={cost} micros, avg_duration={:.1}ms",
            summary["avg_duration_ms"].as_f64().unwrap_or(0.0)
        );
        for result in summary["results"].as_array().into_iter().flatten() {
            println!(
                "  {} passed={} served={} cost={:?}",
                result["id"], result["passed"], result["served_model"], result["cost_micros"]
            );
        }
    }
    Ok(())
}
