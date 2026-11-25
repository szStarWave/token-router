//! `tr-cli admin`（P4-045）：通过 HTTP 管理面访问运行中的服务端。
//!
//! 边界（技术方案 §1.2）：CLI 不直连 SQLite/配置文件做管理查询；
//! 全部经 `tr-client` 走 `/v2/admin/*`（Bearer token）。
//! 本地直连的 `doctor`/`shadow-report` 等离线命令保留在 main.rs。

use clap::Subcommand;
use tr_client::{AdminClient, ClientError, RequestFilter, WindowQuery};

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// 服务状态（版本/监听/schema/digest/模式/计数）
    Status {
        /// JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 候选熔断健康
    Health {
        #[arg(long)]
        json: bool,
    },
    /// 服务端自检（配置/DB/判官/磁盘/日志）
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// 账本请求列表（时间/会话/模型/outcome/协议/原因码过滤）
    Requests {
        /// 时间窗起点（Unix 毫秒；缺省最近 24h）
        #[arg(long)]
        from: Option<i64>,
        #[arg(long)]
        to: Option<i64>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        outcome: Option<String>,
        #[arg(long)]
        protocol: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        json: bool,
    },
    /// 单请求详情
    Request {
        request_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 统计汇总
    Stats {
        #[arg(long)]
        from: Option<i64>,
        #[arg(long)]
        to: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// 时间序列（bucket hour/day + group_by + metric）
    Series {
        #[arg(long, default_value = "hour")]
        bucket: String,
        #[arg(long, default_value = "dispatch_model")]
        group_by: String,
        #[arg(long, default_value = "requests")]
        metric: String,
        #[arg(long)]
        from: Option<i64>,
        #[arg(long)]
        to: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// 配置文档（脱敏）
    Config {
        #[arg(long)]
        json: bool,
    },
    /// 校验配置 TOML（不写盘；返回 issues/diff/digest）
    ConfigValidate {
        /// TOML 文件路径
        document: std::path::PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// 应用配置（CAS；需 If-Match 当前 revision）
    ConfigApply {
        /// TOML 文件路径
        document: std::path::PathBuf,
        /// 当前 revision（`admin config` GET 输出中的 revision；不要用
        /// `config-validate` 返回的候选 digest——CAS 基准是当前磁盘 revision）
        #[arg(long)]
        revision: String,
        #[arg(long)]
        json: bool,
    },
    /// 回滚到最近一份配置备份
    ConfigRollback {
        #[arg(long)]
        json: bool,
    },
    /// 配置审计记录
    Audit {
        #[arg(long)]
        json: bool,
    },
    /// token 列表（脱敏）
    Tokens {
        #[arg(long)]
        json: bool,
    },
    /// 创建 token（明文只显示一次）
    TokenCreate {
        name: String,
        /// 作用域，逗号分隔：inference,admin:read,admin:write,admin:approve
        #[arg(long, default_value = "inference")]
        scopes: String,
        /// 过期时间（Unix 毫秒）
        #[arg(long)]
        expires_at: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// 撤销 token
    TokenRevoke {
        token_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 服务端日志尾
    Logs {
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long)]
        level: Option<String>,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 优雅关闭服务端
    Shutdown {
        #[arg(long)]
        json: bool,
    },
}

/// `--base-url` / `--token` 的公共参数（main.rs 展开传入）。
pub struct AdminArgs {
    pub base_url: String,
    pub token: String,
}

fn client(args: &AdminArgs) -> Result<AdminClient, ClientError> {
    AdminClient::new(&args.base_url, &args.token)
}

fn print(value: &serde_json::Value, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    }
}

/// 分发 admin 子命令。返回 `Ok(Some(json))` 时 main 已自行打印。
pub fn run(
    command: AdminCommand,
    args: &AdminArgs,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async(command, args, json))
}

async fn run_async(
    command: AdminCommand,
    args: &AdminArgs,
    _json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = client(args)?;
    match command {
        AdminCommand::Status { json } => {
            let status = client.status().await?;
            if json {
                print(&serde_json::to_value(&status)?, json);
            } else {
                println!(
                    "v{} api_v{} listen={} uptime={}ms digest={} judge={} cost_gate={} requests={}",
                    status.version,
                    status.api_version,
                    status.listen,
                    status.uptime_ms,
                    &status.config_digest[..16.min(status.config_digest.len())],
                    status.modes.judge,
                    status.modes.cost_gate,
                    status.requests_total,
                );
                println!(
                    "breakers: closed={} half_open={} open={}",
                    status.breakers.closed, status.breakers.half_open, status.breakers.open
                );
            }
        }
        AdminCommand::Health { json } => {
            let health = client.health().await?;
            if json {
                print(&serde_json::to_value(&health)?, json);
            } else {
                for item in &health.items {
                    println!(
                        "{:<20} {:<10} failures={} opened={} last_error={:?}",
                        item.candidate_id,
                        item.status,
                        item.consecutive_failures,
                        item.opened_count,
                        item.last_error_class
                    );
                }
            }
        }
        AdminCommand::Doctor { json } => {
            let doctor = client.doctor().await?;
            if json {
                print(&serde_json::to_value(&doctor)?, json);
            } else {
                for check in &doctor.checks {
                    println!(
                        "[{}] {}: {}",
                        check.status,
                        check.name,
                        check.detail.as_deref().unwrap_or("")
                    );
                }
            }
        }
        AdminCommand::Requests {
            from,
            to,
            session,
            model,
            outcome,
            protocol,
            reason,
            limit,
            json,
        } => {
            let filter = RequestFilter {
                session_key: session,
                model,
                outcome,
                protocol,
                reason_code: reason,
                limit: Some(limit),
                cursor: None,
            };
            let _ = (from, to); // 时间窗过滤在 client 的 RequestFilter 里暂未带窗口；走默认 24h。
            let rows = client.requests(&filter).await?;
            if json {
                print(&serde_json::to_value(&rows)?, json);
            } else {
                for row in &rows {
                    println!(
                        "{} {} {} {} -> {} cost={}ms tokens={}/{}",
                        row.request_id,
                        tr_time(row.created_at),
                        row.selection_model,
                        row.dispatch_model.as_deref().unwrap_or("-"),
                        row.outcome,
                        row.duration_ms,
                        row.tokens.input,
                        row.tokens.output
                    );
                }
            }
        }
        AdminCommand::Request { request_id, json } => {
            let detail = client.request_detail(&request_id).await?;
            if json {
                print(&serde_json::to_value(&detail)?, json);
            } else {
                println!(
                    "{} outcome={} dispatch={:?} plan_mode={} attempts={}",
                    detail.summary.request_id,
                    detail.summary.outcome,
                    detail.summary.dispatch_model,
                    detail.plan_mode,
                    detail.attempts.len()
                );
            }
        }
        AdminCommand::Stats { from, to, json } => {
            let window = WindowQuery { from, to };
            let stats = client.stats_summary(&window).await?;
            if json {
                print(&serde_json::to_value(&stats)?, json);
            } else {
                println!(
                    "requests={} successes={} rate={:.1}% upstream_errors={} switches={}",
                    stats.requests,
                    stats.successes,
                    stats.success_rate * 100.0,
                    stats.upstream_errors,
                    stats.switches
                );
                println!(
                    "tokens: in={} out={} cache_read={} cost: answer={} routing={}",
                    stats.tokens.input,
                    stats.tokens.output,
                    stats.tokens.cache_read,
                    stats.cost_micros.answer,
                    stats.cost_micros.routing
                );
            }
        }
        AdminCommand::Series {
            bucket,
            group_by,
            metric,
            from,
            to,
            json,
        } => {
            let window = WindowQuery { from, to };
            let series = client
                .stats_series(&window, &bucket, &group_by, &metric)
                .await?;
            if json {
                print(&serde_json::to_value(&series)?, json);
            } else {
                for group in &series.series {
                    let points = group
                        .points
                        .iter()
                        .filter(|point| point.value > 0)
                        .map(|point| format!("{}:{}", tr_time(point.t), point.value))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("{:<24} {points}", group.key);
                }
            }
        }
        AdminCommand::Config { json } => {
            let document = client.config().await?;
            if json {
                print(&serde_json::to_value(&document)?, json);
            } else {
                println!("revision: {}", document.revision);
                println!("schema_version: {}", document.schema_version);
                println!(
                    "secrets: {}",
                    document
                        .secrets
                        .iter()
                        .map(|secret| format!("{}={}", secret.path, secret.reference))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!(
                    "document:\n{}",
                    serde_json::to_string_pretty(&document.document)?
                );
            }
        }
        AdminCommand::ConfigValidate { document, json } => {
            let text = std::fs::read_to_string(&document)?;
            let response = client.config_validate(&text).await?;
            if json {
                print(&serde_json::to_value(&response)?, json);
            } else {
                println!("valid: {}", response.valid);
                if let Some(revision) = &response.revision {
                    println!("revision: {revision}");
                }
                for issue in &response.issues {
                    println!("issue [{}] {}: {}", issue.code, issue.path, issue.message);
                }
                for diff in &response.diff {
                    println!("diff {}: {:?} -> {:?}", diff.path, diff.from, diff.to);
                }
            }
        }
        AdminCommand::ConfigApply {
            document,
            revision,
            json,
        } => {
            let text = std::fs::read_to_string(&document)?;
            let request = tr_contracts::ApplyRequest {
                document_toml: text,
                restart: Some("manual".to_string()),
            };
            let response = client.config_apply(&revision, &request).await?;
            if json {
                print(&serde_json::to_value(&response)?, json);
            } else {
                println!(
                    "applied revision {} (restart_required={}; backup={:?})",
                    response.revision, response.restart_required, response.backup_path
                );
            }
        }
        AdminCommand::ConfigRollback { json } => {
            let response = client.config_rollback().await?;
            if json {
                print(&serde_json::to_value(&response)?, json);
            } else {
                println!(
                    "rolled back to revision {} (restart_required={})",
                    response.revision, response.restart_required
                );
            }
        }
        AdminCommand::Audit { json } => {
            let entries = client.config_audit().await?;
            if json {
                print(&serde_json::to_value(&entries)?, json);
            } else {
                for entry in &entries {
                    println!(
                        "{} [{}] {} {} -> {} diff={}",
                        tr_time(entry.at),
                        entry.action,
                        entry.actor,
                        entry.from_revision.as_deref().unwrap_or("-"),
                        entry.to_revision.as_deref().unwrap_or("-"),
                        entry.diff.len()
                    );
                }
            }
        }
        AdminCommand::Tokens { json } => {
            let tokens = client.tokens().await?;
            if json {
                print(&serde_json::to_value(&tokens)?, json);
            } else {
                for token in &tokens {
                    println!(
                        "{} {} {:<10} scopes={:?} revoked={:?}",
                        token.id, token.prefix, token.name, token.scopes, token.revoked_at
                    );
                }
            }
        }
        AdminCommand::TokenCreate {
            name,
            scopes,
            expires_at,
            json,
        } => {
            let scope_list = scopes
                .split(',')
                .filter_map(|scope| tr_contracts::AdminScope::parse(scope.trim()))
                .collect::<Vec<_>>();
            let request = tr_contracts::CreateTokenRequest {
                name,
                scopes: scope_list,
                expires_at,
            };
            let created = client.create_token(&request).await?;
            if json {
                print(&serde_json::to_value(&created)?, json);
            } else {
                println!(
                    "token id={} prefix={}",
                    created.info.id, created.info.prefix
                );
                println!("TOKEN (只显示一次): {}", created.token);
            }
        }
        AdminCommand::TokenRevoke { token_id, json } => {
            client.revoke_token(&token_id).await?;
            if json {
                print(&serde_json::json!({"ok": true, "id": token_id}), json);
            } else {
                println!("revoked {token_id}");
            }
        }
        AdminCommand::Logs {
            limit,
            level,
            target,
            json,
        } => {
            let entries = client.logs(None, Some(limit)).await?;
            let filtered: Vec<_> = entries
                .into_iter()
                .filter(|entry| {
                    level
                        .as_deref()
                        .is_none_or(|level| entry.level.eq_ignore_ascii_case(level))
                        && target
                            .as_deref()
                            .is_none_or(|target| entry.target.starts_with(target))
                })
                .collect();
            if json {
                print(&serde_json::to_value(&filtered)?, json);
            } else {
                for entry in &filtered {
                    println!(
                        "{} [{:<5}] {}: {}",
                        tr_time(entry.at),
                        entry.level,
                        entry.target,
                        entry.message
                    );
                }
            }
        }
        AdminCommand::Shutdown { json } => {
            client.shutdown().await?;
            if json {
                print(&serde_json::json!({"ok": true}), json);
            } else {
                println!("shutdown requested");
            }
        }
    }
    Ok(())
}

/// Unix 毫秒 → 本地可读时间（无 chrono 依赖：只打印 epoch 秒）。
fn tr_time(millis: i64) -> String {
    format!("@{}", millis / 1000)
}
