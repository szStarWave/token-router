//! v2 服务端入口：`token-router-v2 --config <path>`。
//!
//! 配置加载失败、schema 版本不符、端口占用都直接退出并打印原因；
//! 不做“带病启动”（否则用户会以为路由在正常工作）。

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use tr_agent::config;
use tr_agent::pipeline::Pipeline;
use tr_agent::store::Db;
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
        .unwrap_or_else(|| args.config.with_file_name("token-router-v2.db"));
    let db = Db::open(&db_path)?;

    let listen = args.listen.unwrap_or_else(|| catalog.server.listen.clone());
    let addr: SocketAddr = listen
        .parse()
        .map_err(|error| format!("invalid listen address `{listen}`: {error}"))?;

    let auth = if catalog.server.auth_enabled {
        AuthMode::InboundKey(catalog.server.inbound_key.clone())
    } else {
        AuthMode::Disabled
    };
    let admin_token = if catalog.server.admin_token.is_empty() {
        None
    } else {
        Some(catalog.server.admin_token.clone())
    };

    let pipeline = std::sync::Arc::new(Pipeline::from_catalog(db, catalog));
    let app = tr_server::router(AppState::new_shared(pipeline.clone(), auth)).merge(
        tr_control::router(tr_control::ControlState::new(pipeline, admin_token)),
    );

    println!("token-router v2 listening on http://{addr}");
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
