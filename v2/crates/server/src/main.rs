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
use tr_server::{AppState, AuthMode, serve};

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

    let pipeline = Pipeline::from_catalog(db, catalog);
    println!("token-router v2 listening on http://{addr}");

    serve(AppState::new(pipeline, auth), addr).await?;
    Ok(())
}
