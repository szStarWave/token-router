//! v2 服务端入口：`token-router-v2 --config <path>`。
//!
//! 配置加载失败、schema 版本不符、端口占用都直接退出并打印原因；
//! 不做“带病启动”（否则用户会以为路由在正常工作）。
//!
//! P4：管理 API 走 scoped token store（`admin.token` bootstrap）；
//! `POST /v2/admin/shutdown` 触发优雅关闭；内存 ring 记录最近 2000 行日志。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tr_agent::config;
use tr_agent::pipeline::Pipeline;
use tr_agent::store::Db;
use tr_control::{AdminAuth, ControlState, LogRing, LogTail};
use tr_server::{AppState, AuthMode};

#[derive(Debug, Parser)]
#[command(name = "token-router-v2", version, about = "Token Router v2 gateway")]
struct Args {
    /// 配置文件路径（v2 自有 schema）
    #[arg(long)]
    config: PathBuf,

    /// SQLite 路径；默认与配置同目录的 token-router-v2.db
    #[arg(long)]
    db: Option<PathBuf>,

    /// 监听地址覆盖（默认取配置里的 server.listen）
    #[arg(long)]
    listen: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let args = Args::parse();

    let catalog = config::load(&args.config)?;
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| args.config.with_file_name("token-router-v2.db"));
    // 共享句柄：pipeline 与管理面（token store/config audit）用同一个 Arc<Db>。
    let db = Arc::new(Db::open(&db_path)?);

    let listen = args.listen.unwrap_or_else(|| catalog.server.listen.clone());
    let addr: SocketAddr = listen
        .parse()
        .map_err(|error| format!("invalid listen address `{listen}`: {error}"))?;

    let auth = if catalog.server.auth_enabled {
        AuthMode::InboundKey(catalog.server.inbound_key.clone())
    } else {
        AuthMode::Disabled
    };

    let pipeline = Arc::new(Pipeline::from_shared(db.clone(), catalog));

    // 管理面鉴权：scoped token store + admin.token bootstrap。
    let data_dir = args
        .config
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_default();
    let admin_auth = AdminAuth::new(db.clone(), Some(data_dir.join("admin.token")));
    if admin_auth.bootstrap_if_empty()?.is_some() {
        println!(
            "admin token written to {}",
            data_dir.join("admin.token").display()
        );
    }

    // 内存日志 ring（契约 §4.10：最近 2000 行）。
    let logs = LogTail::new(LogRing::new());

    let control_state = ControlState::new(pipeline.clone(), admin_auth)
        .with_config_path(args.config.clone())
        .with_logs(logs);

    let app = tr_server::router(AppState::new_shared(pipeline.clone(), auth))
        .merge(tr_control::router(control_state.clone()));

    println!("token-router v2 listening on http://{addr}");

    // SSE 事件同步（P4-081）：账本/熔断 → 事件总线。
    control_state.spawn_event_sync();

    // 优雅关闭：管理面 `/v2/admin/shutdown` 置位后轮询退出。
    let shutdown_state = control_state.clone();
    tokio::spawn(async move {
        loop {
            if shutdown_state.shutdown_requested() {
                println!("shutdown requested; exiting");
                std::process::exit(0);
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });

    tr_server::serve_router(app, addr).await?;
    Ok(())
}

/// 初始化 tracing（P2-11）。`RUST_LOG` 优先；默认只开本项目的 info。
///
/// 失败（例如外部已安装 subscriber）不影响服务启动。
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("tr_server=info,tr_agent=info,tr_control=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(std::io::stderr)
        .try_init();
}
