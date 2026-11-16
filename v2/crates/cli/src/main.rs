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
    /// 诊断：配置、迁移、判官、账本
    Doctor {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        db: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 任务集评测（成功率/成本/时延）；`eval calibrate` 校准成本闸（P2-9）
    Eval {
        /// 可选子模式：`calibrate` 读观测集校准成本闸，不跑任务
        #[arg(value_name = "MODE")]
        mode: Option<String>,
        /// 校准观测集（JSON 数组，或 `{"observations": [...]}`）
        #[arg(long)]
        observations: Option<PathBuf>,
        /// MAPE 上限
        #[arg(long, default_value_t = 0.25)]
        mape_max: f64,
        /// false deny 上限
        #[arg(long, default_value_t = 0.05)]
        false_deny_max: f64,
        /// 最少样本数；不足则不得 active
        #[arg(long, default_value_t = 30)]
        min_samples: usize,
        #[arg(long)]
        config: Option<PathBuf>,
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
        Command::Doctor { config, db, json } => run_doctor(config, db, json),
        Command::Eval {
            mode,
            observations,
            mape_max,
            false_deny_max,
            min_samples,
            config,
            tasks,
            json,
            db,
        } => {
            if mode.as_deref() == Some("calibrate") {
                let path = observations.ok_or("eval calibrate 需要 --observations <path>")?;
                return run_calibrate(&path, mape_max, false_deny_max, min_samples, json);
            }
            let config = config.ok_or("eval 需要 --config <path>")?;
            run_eval(config, tasks, json, db)
        }
    }
}

fn run_doctor(
    config: PathBuf,
    db_path: Option<PathBuf>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut checks: Vec<serde_json::Value> = Vec::new();
    let mut failed = false;

    let catalog = match tr_agent::config::load(&config) {
        Ok(catalog) => {
            checks.push(serde_json::json!({
                "check": "config",
                "ok": true,
                "detail": format!(
                    "{} candidates, {} virtual models, digest={}",
                    catalog.candidates.len(),
                    catalog.virtual_models.len(),
                    catalog.config_digest
                ),
            }));
            Some(catalog)
        }
        Err(error) => {
            failed = true;
            checks.push(serde_json::json!({
                "check": "config",
                "ok": false,
                "detail": error.to_string(),
            }));
            None
        }
    };

    let db_path = db_path.unwrap_or_else(|| config.with_file_name("token-router-v2.db"));
    match tr_agent::store::Db::open(&db_path) {
        Ok(db) => {
            let version = db.schema_version().unwrap_or(-1);
            checks.push(serde_json::json!({
                "check": "database",
                "ok": version > 0,
                "detail": format!("path={} schema_version={version}", db_path.display()),
            }));
            match tr_agent::store::stats::stats(&db) {
                Ok(stats) => checks.push(serde_json::json!({
                    "check": "ledger",
                    "ok": true,
                    "detail": format!(
                        "requests={} successes={} cost_micros={}",
                        stats.requests, stats.successes, stats.cost_micros()
                    ),
                })),
                Err(error) => {
                    failed = true;
                    checks.push(serde_json::json!({
                        "check": "ledger",
                        "ok": false,
                        "detail": error.to_string(),
                    }));
                }
            }
        }
        Err(error) => {
            failed = true;
            checks.push(serde_json::json!({
                "check": "database",
                "ok": false,
                "detail": error.to_string(),
            }));
        }
    }

    if let Some(catalog) = &catalog {
        let mode = catalog.judge.mode;
        if mode == tr_agent::judge::JudgeMode::Off {
            checks.push(serde_json::json!({
                "check": "judge",
                "ok": true,
                "detail": "off (skipped)",
            }));
        } else {
            match tr_agent::judge::JudgeRuntime::new(catalog.judge.clone()) {
                Ok(runtime) => {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let result = rt.block_on(runtime.self_test());
                    let ok = result.outcome == tr_contracts::TestOutcome::Passed;
                    failed |= !ok;
                    checks.push(serde_json::json!({
                        "check": "judge",
                        "ok": ok,
                        "detail": format!("{:?} ({}ms)", result.outcome, result.latency_ms),
                    }));
                }
                Err(error) => {
                    failed = true;
                    checks.push(serde_json::json!({
                        "check": "judge",
                        "ok": false,
                        "detail": error,
                    }));
                }
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": !failed,
                "checks": checks,
            }))?
        );
    } else {
        for check in &checks {
            println!(
                "[{}] {}: {}",
                if check["ok"] == serde_json::Value::Bool(true) {
                    "ok"
                } else {
                    "FAIL"
                },
                check["check"].as_str().unwrap_or("?"),
                check["detail"].as_str().unwrap_or("")
            );
        }
    }

    if failed {
        return Err("doctor found failures".into());
    }
    Ok(())
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

/// 成本闸校准观测（P2-9）。单位统一为 micros，币种由调用方保证一致。
#[derive(Debug, Clone, serde::Deserialize)]
struct CalibrationObservation {
    #[serde(default)]
    estimated_delta_micros: f64,
    #[serde(default)]
    actual_delta_micros: f64,
    #[serde(default)]
    denied: bool,
    /// 被拒绝的切换事后是否本会成功（false deny 的判据）。
    #[serde(default)]
    would_have_succeeded: bool,
}

/// 校准报告。
#[derive(Debug, Clone, PartialEq)]
struct CalibrationReport {
    samples: usize,
    mape: Option<f64>,
    denied: usize,
    false_denies: usize,
    false_deny_rate: Option<f64>,
    mape_max: f64,
    false_deny_max: f64,
    min_samples: usize,
    passed: bool,
}

impl CalibrationReport {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "samples": self.samples,
            "mape": self.mape,
            "denied": self.denied,
            "false_denies": self.false_denies,
            "false_deny_rate": self.false_deny_rate,
            "thresholds": {
                "mape_max": self.mape_max,
                "false_deny_max": self.false_deny_max,
                "min_samples": self.min_samples,
            },
            "passed": self.passed,
            // 不达标时成本闸保持 shadow（§9.4）。
            "cost_gate_mode": if self.passed { "active_allowed" } else { "shadow" },
        })
    }
}

/// 计算 MAPE 与 false deny。真实 delta 为 0 的样本不参与 MAPE（分母无意义）。
fn calibrate(
    observations: &[CalibrationObservation],
    mape_max: f64,
    false_deny_max: f64,
    min_samples: usize,
) -> CalibrationReport {
    let comparable: Vec<f64> = observations
        .iter()
        .filter(|observation| observation.actual_delta_micros.abs() > f64::EPSILON)
        .map(|observation| {
            (observation.estimated_delta_micros - observation.actual_delta_micros).abs()
                / observation.actual_delta_micros.abs()
        })
        .collect();
    let mape = if comparable.is_empty() {
        None
    } else {
        Some(comparable.iter().sum::<f64>() / comparable.len() as f64)
    };

    let denied: Vec<&CalibrationObservation> = observations
        .iter()
        .filter(|observation| observation.denied)
        .collect();
    let false_denies = denied
        .iter()
        .filter(|observation| observation.would_have_succeeded)
        .count();
    let false_deny_rate = if denied.is_empty() {
        None
    } else {
        Some(false_denies as f64 / denied.len() as f64)
    };

    let passed = observations.len() >= min_samples
        && mape.is_some_and(|mape| mape <= mape_max)
        && false_deny_rate.unwrap_or(0.0) <= false_deny_max;

    CalibrationReport {
        samples: observations.len(),
        mape,
        denied: denied.len(),
        false_denies,
        false_deny_rate,
        mape_max,
        false_deny_max,
        min_samples,
        passed,
    }
}

fn run_calibrate(
    path: &std::path::Path,
    mape_max: f64,
    false_deny_max: f64,
    min_samples: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut object) => object
            .remove("observations")
            .and_then(|value| value.as_array().cloned())
            .ok_or("观测集缺少 observations 数组")?,
        _ => return Err("观测集必须是数组或 {observations: [...]}".into()),
    };
    let observations: Vec<CalibrationObservation> = items
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()?;

    let report = calibrate(&observations, mape_max, false_deny_max, min_samples);
    if json {
        println!("{}", serde_json::to_string_pretty(&report.to_json())?);
    } else {
        println!(
            "calibrate: samples={} mape={:?} false_deny={:?} passed={} gate={}",
            report.samples,
            report.mape,
            report.false_deny_rate,
            report.passed,
            if report.passed {
                "active_allowed"
            } else {
                "shadow"
            }
        );
        if report.samples < report.min_samples {
            println!(
                "  insufficient samples: need >= {}, got {}",
                report.min_samples, report.samples
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        estimated: f64,
        actual: f64,
        denied: bool,
        would_have_succeeded: bool,
    ) -> CalibrationObservation {
        CalibrationObservation {
            estimated_delta_micros: estimated,
            actual_delta_micros: actual,
            denied,
            would_have_succeeded,
        }
    }

    #[test]
    fn calibration_passes_within_thresholds() {
        let observations: Vec<_> = (0..30)
            .map(|index| observation(100.0 + index as f64, 110.0 + index as f64, false, false))
            .collect();
        let report = calibrate(&observations, 0.25, 0.05, 30);
        assert!(report.passed, "{report:?}");
        assert!(report.mape.expect("mape") < 0.25);
        assert_eq!(report.denied, 0);
    }

    #[test]
    fn calibration_fails_on_false_deny_and_low_samples() {
        let observations = vec![
            observation(100.0, 100.0, true, true),
            observation(100.0, 100.0, true, false),
        ];
        let report = calibrate(&observations, 0.25, 0.05, 30);
        assert!(!report.passed);
        assert_eq!(report.false_deny_rate, Some(0.5));
        assert_eq!(report.samples, 2);

        // 足够样本但 false deny 超标也不通过。
        let mut many: Vec<_> = (0..30)
            .map(|index| observation(100.0 + index as f64, 110.0 + index as f64, false, false))
            .collect();
        many.push(observation(100.0, 100.0, true, true));
        let report = calibrate(&many, 0.25, 0.05, 30);
        assert!(!report.passed);
    }

    #[test]
    fn zero_actual_delta_is_excluded_from_mape() {
        let observations = vec![observation(50.0, 0.0, false, false)];
        let report = calibrate(&observations, 0.25, 0.05, 1);
        assert_eq!(report.mape, None);
        assert!(!report.passed, "no comparable samples cannot calibrate");
    }
}
