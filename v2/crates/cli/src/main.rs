//! `tr-cli`：离线命令（replay/check-config/doctor/eval）+ `admin` 子命令（HTTP 管理面）。

mod admin;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "tr-cli", version, about = "Token Router v2 运维命令")]
struct Args {
    /// 管理面 base URL（admin 子命令用；默认本地 17621）
    #[arg(long, global = true, default_value = "http://127.0.0.1:17621")]
    base_url: String,
    /// 管理 token（或环境变量 TR_ADMIN_TOKEN）
    #[arg(long, global = true, env = "TR_ADMIN_TOKEN", hide_env_values = true)]
    token: Option<String>,
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
        /// JSON 输出（等价 compact）
        #[arg(long)]
        json: bool,
    },
    /// 校验配置文件并打印目录摘要
    CheckConfig {
        #[arg(long)]
        config: PathBuf,
        /// JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// shadow 记录聚合（一致率/延迟/P95/预算）
    ShadowReport {
        #[arg(long)]
        db: PathBuf,
        /// JSON 输出
        #[arg(long)]
        json: bool,
        /// active 门槛：最少样本数（P1-13 要求 ≥500）
        #[arg(long, default_value_t = 0)]
        min_samples: i64,
        /// 样本不足时以非零码退出（用于 dogfood 门禁）
        #[arg(long)]
        require: bool,
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
        /// 把三列对照表写入 markdown（P2-8）
        #[arg(long)]
        out: Option<PathBuf>,
        /// 单模型/基线配置；提供后同任务集跑两遍做对照
        #[arg(long)]
        baseline_config: Option<PathBuf>,
        /// 关闭流式（默认流式，才能测首 token 延迟）
        #[arg(long)]
        no_stream: bool,
    },
    /// 运行中服务的管理查询（HTTP 管理面；需要 --token）
    Admin {
        #[command(subcommand)]
        command: admin::AdminCommand,
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

/// 从常见数据目录读 bootstrap token（`~/.token-router-v2/admin.token`）。
fn read_bootstrap_token() -> Option<String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let path = std::path::Path::new(&home)
        .join(".token-router-v2")
        .join("admin.token");
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Admin { command } => {
            let token = args.token.clone().or_else(read_bootstrap_token).ok_or(
                "需要管理 token：--token <token> 或 TR_ADMIN_TOKEN 环境变量，\
                     或 <data_dir>/admin.token 文件",
            )?;
            admin::run(
                command,
                &admin::AdminArgs {
                    base_url: args.base_url.clone(),
                    token,
                },
                false,
            )
        }
        Command::Replay {
            snapshot,
            digest,
            compact,
            json,
        } => {
            if digest {
                let text = std::fs::read_to_string(&snapshot)?;
                let snapshot: tr_contracts::DecisionSnapshot = serde_json::from_str(&text)?;
                println!("{}", tr_cli::replay_digest(&snapshot)?);
                return Ok(());
            }
            let plan = tr_cli::replay_file(&snapshot)?;
            if compact || json {
                println!("{}", serde_json::to_string(&plan)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
            Ok(())
        }
        Command::CheckConfig { config, json } => {
            let catalog = tr_agent::config::load(&config)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "ok": true,
                        "candidates": catalog.candidates.len(),
                        "virtual_models": catalog.virtual_models.len(),
                        "default_virtual_model": catalog.default_virtual_model,
                        "config_digest": catalog.config_digest,
                        "judge_mode": format!("{:?}", catalog.judge.mode),
                        "cost_gate": format!("{:?}", catalog.cost_gate),
                        "trust_tiers": catalog.trust_tiers,
                    }))?
                );
            } else {
                println!(
                    "ok: {} candidates, {} virtual models, default = {}, config_digest = {}",
                    catalog.candidates.len(),
                    catalog.virtual_models.len(),
                    catalog.default_virtual_model,
                    catalog.config_digest
                );
            }
            Ok(())
        }
        Command::ShadowReport {
            db,
            json,
            min_samples,
            require,
        } => {
            let db = tr_agent::store::Db::open(&db)?;
            let summary = tr_agent::store::judge::shadow_summary(&db)?;
            let enough = summary.total >= min_samples;
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
                        "p95_latency_ms": summary.p95_latency_ms,
                        "total_cost_micros": summary.total_cost_micros,
                        "min_samples": min_samples,
                        "enough_samples": enough,
                    }))?
                );
            } else {
                println!(
                    "shadow: total={} agreed={} disagreed={} unknown={} rate={:?} avg_latency_ms={:?} p95_latency_ms={:?} cost_micros={}",
                    summary.total,
                    summary.agreed,
                    summary.disagreed,
                    summary.unknown,
                    summary.agreement_rate(),
                    summary.avg_latency_ms,
                    summary.p95_latency_ms,
                    summary.total_cost_micros
                );
                if !enough {
                    println!(
                        "  samples below minimum: {} < {} (judge must stay shadow/off)",
                        summary.total, min_samples
                    );
                }
            }
            if require && !enough {
                return Err(format!(
                    "shadow samples {} below required {}",
                    summary.total, min_samples
                )
                .into());
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
            out,
            baseline_config,
            no_stream,
        } => {
            if mode.as_deref() == Some("calibrate") {
                let path = observations.ok_or("eval calibrate 需要 --observations <path>")?;
                return run_calibrate(&path, mape_max, false_deny_max, min_samples, json);
            }
            let config = config.ok_or("eval 需要 --config <path>")?;
            run_eval(config, tasks, json, db, out, baseline_config, !no_stream)
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

#[derive(Debug, Clone, serde::Deserialize)]
struct EvalTask {
    id: String,
    /// `simple` | `medium` | `hard` | `tool`（P2-7 覆盖要求）。
    #[serde(default)]
    difficulty: Option<String>,
    prompt: String,
    expect: EvalExpect,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct EvalExpect {
    /// 文本包含即通过。
    #[serde(default)]
    contains: Option<String>,
    /// 响应中出现指定工具调用即通过（tool 类任务）。
    #[serde(default)]
    tool: Option<String>,
}

impl EvalExpect {
    fn matched(&self, text: &str, tool_names: &[String]) -> bool {
        let text_ok = self
            .contains
            .as_deref()
            .is_some_and(|needle| text.contains(needle));
        let tool_ok = self
            .tool
            .as_deref()
            .is_some_and(|needle| tool_names.iter().any(|name| name == needle));
        text_ok || tool_ok
    }
}

#[derive(Debug, Clone)]
struct EvalRun {
    name: String,
    tasks: usize,
    passed: usize,
    total_cost_micros: i64,
    avg_ttft_ms: Option<f64>,
    avg_duration_ms: Option<f64>,
    results: Vec<serde_json::Value>,
}

impl EvalRun {
    fn success_rate(&self) -> Option<f64> {
        (self.tasks > 0).then(|| self.passed as f64 / self.tasks as f64)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "tasks": self.tasks,
            "passed": self.passed,
            "success_rate": self.success_rate(),
            "total_cost_micros": self.total_cost_micros,
            "avg_ttft_ms": self.avg_ttft_ms,
            "avg_duration_ms": self.avg_duration_ms,
            "results": self.results,
        })
    }
}

fn run_eval(
    config: PathBuf,
    tasks_path: PathBuf,
    json: bool,
    db_path: Option<PathBuf>,
    out: Option<PathBuf>,
    baseline_config: Option<PathBuf>,
    stream: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let tasks_text = std::fs::read_to_string(&tasks_path)?;
    let tasks: Vec<EvalTask> = serde_json::from_str::<serde_json::Value>(&tasks_text)?
        .get("tasks")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .ok_or("tasks file 缺少 tasks 数组")?;

    let mut runs = vec![evaluate_catalog(
        "router",
        &config,
        &tasks,
        db_path.clone(),
        stream,
    )?];
    if let Some(baseline) = &baseline_config {
        runs.push(evaluate_catalog(
            "baseline", baseline, &tasks, None, stream,
        )?);
    } else {
        eprintln!("note: 未声明 --baseline-config；按 §11.5 不能据此比较成本/质量收益");
    }

    let table = render_eval_table(&runs);
    if let Some(path) = &out {
        let baseline = baseline_config
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "未声明".to_string());
        let document = format!(
            "# P2 评测对照\n\n\
             - 任务集: `{}`\n\
             - 任务数: {}\n\
             - 基线: `{}`\n\
             - 模式: {}\n\n\
             {}\n\n\
             > 每行是同一任务集的一次完整运行；成功率由 checker 判定，等价成本 = answer + routing，首 token 延迟取流式首个事件。\n",
            tasks_path.display(),
            tasks.len(),
            baseline,
            if stream { "stream" } else { "non-stream" },
            table
        );
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, document)?;
    }

    if json {
        let summary = serde_json::json!({
            "tasks_path": tasks_path.display().to_string(),
            "stream": stream,
            "runs": runs.iter().map(EvalRun::to_json).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!("{table}");
        for run in &runs {
            for result in &run.results {
                println!(
                    "  [{}] {} passed={} served={} cost={:?} ttft={:?}",
                    run.name,
                    result["id"],
                    result["passed"],
                    result["served_model"],
                    result["cost_micros"],
                    result["ttft_ms"]
                );
            }
        }
    }
    Ok(())
}

fn render_eval_table(runs: &[EvalRun]) -> String {
    let mut out = String::from(
        "| 配置 | 成功率 | 等价成本 (micros) | 首 token 延迟 (ms) |\n\
         |---|---|---|---|\n",
    );
    for run in runs {
        let rate = run
            .success_rate()
            .map(|rate| format!("{:.1}%", rate * 100.0))
            .unwrap_or_else(|| "—".to_string());
        let ttft = run
            .avg_ttft_ms
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "—".to_string());
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            run.name, rate, run.total_cost_micros, ttft
        ));
    }
    out
}

fn evaluate_catalog(
    name: &str,
    config: &std::path::Path,
    tasks: &[EvalTask],
    db_path: Option<PathBuf>,
    stream: bool,
) -> Result<EvalRun, Box<dyn std::error::Error>> {
    let catalog = tr_agent::config::load(config)?;
    let db_path = db_path.unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "tr-eval-{name}-{}.db",
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
    let mut ttfts: Vec<f64> = Vec::new();
    let mut durations: Vec<f64> = Vec::new();

    for task in tasks {
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

        if stream {
            let mut streaming = match runtime.block_on(
                pipeline.handle_stream(request, tr_agent::pipeline::RequestContext::default()),
            ) {
                Ok(streaming) => streaming,
                Err(error) => {
                    results.push(serde_json::json!({
                        "id": task.id,
                        "difficulty": task.difficulty,
                        "passed": false,
                        "error": error.to_string(),
                    }));
                    continue;
                }
            };

            let started = std::time::Instant::now();
            let mut text = String::new();
            let mut tool_names: Vec<String> = Vec::new();
            let mut ttft_ms: Option<u64> = None;
            let mut failed: Option<String> = None;

            while let Some(chunk) = runtime.block_on(streaming.events.recv()) {
                match chunk {
                    tr_agent::pipeline::StreamChunk::Event(event) => {
                        let elapsed_ms = started.elapsed().as_millis() as u64;
                        match event {
                            tr_contracts::CanonicalStreamEvent::ContentDelta { text: delta } => {
                                // 首 token = 第一个内容帧；Started/Usage 不算。
                                if ttft_ms.is_none() {
                                    ttft_ms = Some(elapsed_ms);
                                }
                                text.push_str(&delta);
                            }
                            tr_contracts::CanonicalStreamEvent::ToolCallDelta { name, .. } => {
                                if ttft_ms.is_none() {
                                    ttft_ms = Some(elapsed_ms);
                                }
                                if let Some(name) = name {
                                    tool_names.push(name);
                                }
                            }
                            tr_contracts::CanonicalStreamEvent::Error { message } => {
                                failed = Some(message);
                            }
                            _ => {}
                        }
                    }
                    tr_agent::pipeline::StreamChunk::Done => break,
                    tr_agent::pipeline::StreamChunk::Error { message } => {
                        failed = Some(message);
                        break;
                    }
                }
            }
            let duration_ms = started.elapsed().as_millis() as f64;
            let served_model = streaming.headers.served_model.clone();
            let plan_id = streaming.headers.plan_id.clone();
            let cost_micros = wait_for_eval_cost(&pipeline, &plan_id);
            let passed = failed.is_none() && task.expect.matched(&text, &tool_names);

            if let Some(ttft) = ttft_ms {
                ttfts.push(ttft as f64);
            }
            durations.push(duration_ms);
            results.push(serde_json::json!({
                "id": task.id,
                "difficulty": task.difficulty,
                "passed": passed,
                "served_model": served_model,
                "cost_micros": cost_micros,
                "ttft_ms": ttft_ms,
                "duration_ms": duration_ms as u64,
                "error": failed,
            }));
        } else {
            match runtime
                .block_on(pipeline.handle(request, tr_agent::pipeline::RequestContext::default()))
            {
                Ok(result) => {
                    let mut text = String::new();
                    let mut tool_names = Vec::new();
                    for block in &result.response.blocks {
                        match block {
                            tr_contracts::ContentBlock::Text { text: chunk, .. } => {
                                text.push_str(chunk);
                            }
                            tr_contracts::ContentBlock::ToolUse { name, .. } => {
                                tool_names.push(name.clone());
                            }
                            _ => {}
                        }
                    }
                    durations.push(result.ledger.duration_ms as f64);
                    results.push(serde_json::json!({
                        "id": task.id,
                        "difficulty": task.difficulty,
                        "passed": task.expect.matched(&text, &tool_names),
                        "served_model": result.headers.served_model,
                        "cost_micros": result.ledger.cost_equiv.as_ref().map(|cost| cost.micros),
                        "ttft_ms": result.ledger.ttft_ms,
                        "duration_ms": result.ledger.duration_ms,
                    }));
                }
                Err(error) => {
                    results.push(serde_json::json!({
                        "id": task.id,
                        "difficulty": task.difficulty,
                        "passed": false,
                        "error": error.to_string(),
                    }));
                }
            }
        }
    }

    let total = results.len();
    let passed = results
        .iter()
        .filter(|result| result["passed"] == serde_json::Value::Bool(true))
        .count();
    let total_cost_micros = results
        .iter()
        .filter_map(|result| result["cost_micros"].as_i64())
        .sum();

    Ok(EvalRun {
        name: name.to_string(),
        tasks: total,
        passed,
        total_cost_micros,
        avg_ttft_ms: average(&ttfts),
        avg_duration_ms: average(&durations),
        results,
    })
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// 流式账本在后台提交；给最多 2s，拿不到就返回 `None`（不虚构成本）。
fn wait_for_eval_cost(pipeline: &tr_agent::pipeline::Pipeline, plan_id: &str) -> Option<i64> {
    for _ in 0..200 {
        match tr_agent::store::ledger::cost_for_plan(pipeline.db(), plan_id) {
            Ok(Some(cost)) => return Some(cost),
            Ok(None) => {}
            Err(_) => return None,
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
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

    #[test]
    fn expect_matches_text_or_tool() {
        let text_expect = EvalExpect {
            contains: Some("Paris".to_string()),
            tool: None,
        };
        assert!(text_expect.matched("the capital is Paris", &[]));
        assert!(!text_expect.matched("Lyon", &[]));

        let tool_expect = EvalExpect {
            contains: None,
            tool: Some("read_file".to_string()),
        };
        assert!(tool_expect.matched("", &["read_file".to_string()]));
        assert!(!tool_expect.matched("", &["list_files".to_string()]));

        assert!(!EvalExpect::default().matched("anything", &[]));
    }

    #[test]
    fn eval_table_has_three_metric_columns() {
        let runs = vec![
            EvalRun {
                name: "router".to_string(),
                tasks: 30,
                passed: 27,
                total_cost_micros: 1234,
                avg_ttft_ms: Some(120.5),
                avg_duration_ms: Some(900.0),
                results: Vec::new(),
            },
            EvalRun {
                name: "baseline".to_string(),
                tasks: 30,
                passed: 24,
                total_cost_micros: 2000,
                avg_ttft_ms: Some(90.0),
                avg_duration_ms: Some(700.0),
                results: Vec::new(),
            },
        ];
        let table = render_eval_table(&runs);
        assert!(table.contains("| 配置 | 成功率 | 等价成本 (micros) | 首 token 延迟 (ms) |"));
        assert!(table.contains("| router | 90.0% | 1234 | 120.5 |"));
        assert!(table.contains("| baseline | 80.0% | 2000 | 90.0 |"));
    }

    #[test]
    fn eval_task_set_meets_p2_7_acceptance() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/eval/tasks.json");
        let text = std::fs::read_to_string(&path).expect("tasks.json");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        let tasks = value["tasks"].as_array().expect("tasks").clone();
        assert!(tasks.len() >= 30, "P2-7 requires >= 30 tasks");
        let mut difficulties = std::collections::BTreeSet::new();
        for task in &tasks {
            let difficulty = task["difficulty"].as_str().unwrap_or("missing");
            assert!(
                matches!(difficulty, "simple" | "medium" | "hard" | "tool"),
                "unexpected difficulty `{difficulty}`"
            );
            difficulties.insert(difficulty.to_string());
        }
        for required in ["simple", "medium", "hard", "tool"] {
            assert!(
                difficulties.contains(required),
                "missing difficulty coverage: {required}"
            );
        }
    }
}
