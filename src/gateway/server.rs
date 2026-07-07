use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::time::Instant;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::gateway::api::compat::CodexChatHistoryStore;
use crate::gateway::api::routes::{router, AppState};
use crate::gateway::config::AppConfig;
use crate::gateway::config_manager::ConfigManager;
use crate::gateway::daemon;
use crate::config::sessions_dir;

use crate::gateway::classifier::ClassifierStore;
use crate::gateway::experience::ExperienceStore;
use crate::gateway::multimodal::MultimodalStore;
use crate::gateway::agent_usage::AgentCloudUsageStore;
use crate::gateway::session::SessionStore;
use crate::gateway::stats::GatewayStats;
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::routing::{AdaptiveTuner, compute_effective_routing, WordFreqStore};
use crate::gateway::routing_log::RoutingLogStore;
use crate::gateway::upstream::UpstreamClient;

pub struct GatewayRuntime {
    pub started_at: Instant,
    pub started_at_unix: u64,
    shutdown: watch::Sender<bool>,
}

impl GatewayRuntime {
    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    pub fn trigger_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }
}

#[derive(Debug)]
pub struct RunOptions {
    pub register_pid: bool,
    pub listen_for_ctrl_c: bool,
    pub cancel: CancellationToken,
    pub ready: Option<SyncSender<()>>,
}

impl RunOptions {
    pub fn daemon(register_pid: bool) -> Self {
        Self {
            register_pid,
            listen_for_ctrl_c: true,
            cancel: CancellationToken::new(),
            ready: None,
        }
    }

    pub fn embedded(cancel: CancellationToken, ready: SyncSender<()>) -> Self {
        Self {
            register_pid: false,
            listen_for_ctrl_c: false,
            cancel,
            ready: Some(ready),
        }
    }
}

pub async fn run(config: AppConfig, register_pid: bool) -> anyhow::Result<()> {
    run_with_options(config, RunOptions::daemon(register_pid)).await
}

pub async fn run_with_options(config: AppConfig, opts: RunOptions) -> anyhow::Result<()> {
    if opts.register_pid {
        daemon::ensure_data_dir(&config)?;
        daemon::write_pid_file(&config)?;
    }

    let cancel = opts.cancel.clone();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let runtime = Arc::new(GatewayRuntime {
        started_at: Instant::now(),
        started_at_unix: daemon::started_at_unix(),
        shutdown: shutdown_tx,
    });

    let stats = GatewayStats::open(&config.data_dir)?;
    stats.spawn_flush_task();

    let experience = ExperienceStore::open(&config.data_dir, config.experience.clone())?;
    experience.spawn_flush_task();

    let classifier = ClassifierStore::open(&config.data_dir, config.classifier.clone())?;
    classifier.spawn_flush_task();
    classifier.spawn_decay_task();

    let multimodal = MultimodalStore::open(&config.data_dir)?;
    multimodal.spawn_flush_task();

    let sessions_path = sessions_dir().unwrap_or_else(|_| config.data_dir.join("sessions"));
    let sessions = SessionStore::open(sessions_path, config.session_persist_enabled)?;
    sessions.spawn_flush_task();
    if config.session_persist_enabled {
        sessions.spawn_cleanup_task(
            config.session_retention_days,
            config.session_cleanup_interval_secs,
        );
    }

    let agent_usage = AgentCloudUsageStore::open(&config.data_dir)?;
    agent_usage.spawn_flush_task();

    let wordfreq = WordFreqStore::open(&config.data_dir, config.wordfreq.clone())?;
    wordfreq.spawn_flush_task();

    let routing_logs = RoutingLogStore::open(&config.data_dir)?;
    routing_logs.spawn_route_cache_cleanup_task(
        config.request_route_cache_retention_days,
        config.request_route_cache_cleanup_interval_secs,
    );

    let sessions_for_shutdown = sessions.clone();
    let experience_for_shutdown = experience.clone();
    let classifier_for_shutdown = classifier.clone();
    let multimodal_for_shutdown = multimodal.clone();
    let agent_usage_for_shutdown = agent_usage.clone();
    let multimodal_for_upstream = multimodal.clone();
    let adaptive_tuner = Arc::new(AdaptiveTuner::new(compute_effective_routing(&config)));
    let edge_load = EdgeInferenceTracker::new();
    let config_mgr = ConfigManager::new(config.clone());
    let codex_history = Arc::new(CodexChatHistoryStore::default());
    let state = AppState {
        config_mgr: config_mgr.clone(),
        sessions: sessions.clone(),
        experience,
        classifier,
        multimodal,
        upstream: UpstreamClient::new(
            config_mgr,
            stats.clone(),
            multimodal_for_upstream,
            edge_load.clone(),
            agent_usage.clone(),
            routing_logs.clone(),
            sessions.clone(),
        ),
        runtime: runtime.clone(),
        stats: stats.clone(),
        adaptive_tuner,
        edge_load,
        agent_usage,
        wordfreq,
        routing_logs,
        codex_history,
    };

    let app = router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    let addr: SocketAddr = config.listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;

    if let Some(ready) = opts.ready {
        let _ = ready.send(());
    }

    info!(%addr, "token-router gateway listening");
    info!(
        edge = config.edge_base_url.is_some(),
        cloud = config.cloud_base_url.is_some(),
        profile = ?config.default_profile,
        pid_file = %config.pid_file.display(),
        "gateway ready"
    );

    let cancel_serve = cancel.clone();
    let serve = async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                cancel_serve.cancelled().await;
            })
            .await
    };

    let mut shutdown_rx = shutdown_rx;
    let cancel_shutdown = cancel.clone();
    tokio::spawn(async move {
        let _ = shutdown_rx.changed().await;
        cancel_shutdown.cancel();
    });

    if opts.listen_for_ctrl_c {
        let ctrl_c = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("ctrl-c received, shutting down");
                ctrl_c.cancel();
            }
        });
    }

    serve.await?;

    if let Err(e) = stats.flush() {
        tracing::warn!(error = %e, "final stats flush failed");
    }
    if let Err(e) = experience_for_shutdown.flush() {
        tracing::warn!(error = %e, "final experience flush failed");
    }
    if let Err(e) = classifier_for_shutdown.flush() {
        tracing::warn!(error = %e, "final classifier flush failed");
    }
    if let Err(e) = multimodal_for_shutdown.flush() {
        tracing::warn!(error = %e, "final multimodal capability flush failed");
    }
    if let Err(e) = sessions_for_shutdown.flush() {
        tracing::warn!(error = %e, "final session flush failed");
    }
    if let Err(e) = agent_usage_for_shutdown.flush() {
        tracing::warn!(error = %e, "final agent_usage flush failed");
    }

    if opts.register_pid {
        daemon::remove_pid_file(&config);
    }

    Ok(())
}

#[allow(dead_code)]
pub fn app_router(state: AppState) -> Router {
    router(state)
}
