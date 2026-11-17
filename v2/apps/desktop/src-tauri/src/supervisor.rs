//! Supervisor（技术方案 §6.2）：spawn / 健康检查 / 退避重启 / 孤儿清理。
//!
//! 状态机：
//! ```text
//! Stopped --start--> Starting --ready--> Ready
//!    ^                  |                 | crash
//!    |                  | timeout/fail    v
//!    +--------------- Failed <-------- Restarting
//!                                       +--backoff--> Starting
//! ```
//!
//! 约束：
//! - 端口默认 17621；占用则 17622..17640；
//! - 就绪 = `GET /health` 2xx；超时 15s；
//! - 崩溃退避 1s/2s/4s/8s/16s，最多 5 次 → Failed；
//! - `runtime.json` 用于孤儿进程发现。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Child;
use tokio::sync::Mutex;

/// supervisor 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionState {
    Stopped,
    Starting,
    Ready,
    Restarting,
    Failed,
}

impl SupervisionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Restarting => "restarting",
            Self::Failed => "failed",
        }
    }
}

/// supervisor 配置。
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// sidecar 可执行文件路径（`token-router-v2`）。
    pub sidecar_path: PathBuf,
    /// 数据目录（`config.toml`、DB、`admin.token`、`runtime.json` 所在）。
    pub data_dir: PathBuf,
    /// 默认监听地址。
    pub listen: String,
    /// 外部模式：不 spawn，只连接。
    pub external: Option<String>,
}

/// supervisor 运行时。
pub struct Supervisor {
    pub config: SupervisorConfig,
    state: Arc<Mutex<SupervisionState>>,
    child: Arc<Mutex<Option<Child>>>,
    listen_addr: Arc<Mutex<Option<String>>>,
    restarts: Arc<Mutex<u32>>,
}

/// 单次 spawn 尝试的结果。
pub struct SpawnOutcome {
    pub state: SupervisionState,
    pub listen: Option<String>,
    pub message: String,
}

impl Supervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(SupervisionState::Stopped)),
            child: Arc::new(Mutex::new(None)),
            listen_addr: Arc::new(Mutex::new(None)),
            restarts: Arc::new(Mutex::new(0)),
        }
    }

    pub async fn state(&self) -> SupervisionState {
        *self.state.lock().await
    }

    pub async fn listen_addr(&self) -> Option<String> {
        self.listen_addr.lock().await.clone()
    }

    /// 就绪探测：`GET {base}/health` 2xx。
    async fn wait_ready(base: &str, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            let url = format!("{base}/health");
            let ready = match tokio::time::timeout(Duration::from_secs(2), reqwest_get(&url)).await
            {
                Ok(Ok(response)) => response.status().is_success(),
                _ => false,
            };
            if ready {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        false
    }

    /// 选择空闲端口：17621 起递增到 17640。
    async fn pick_port(listen: &str) -> Option<String> {
        let default_port = listen.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(17621);
        let host = listen
            .rsplit_once(':')
            .map(|(host, _)| host.to_string())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        for port in default_port..=17640 {
            let candidate = format!("{host}:{port}");
            let addr: SocketAddr = candidate.parse().ok()?;
            if tokio::net::TcpListener::bind(addr).await.is_ok() {
                return Some(candidate);
            }
        }
        None
    }

    /// 启动（或连接外部）服务端；就绪后写 `runtime.json`。
    pub async fn start(&self) -> SpawnOutcome {
        // 外部模式：只探测可达性。
        if let Some(base) = &self.config.external {
            *self.state.lock().await = SupervisionState::Starting;
            if Self::wait_ready(base, Duration::from_secs(15)).await {
                *self.listen_addr.lock().await = Some(base.clone());
                *self.state.lock().await = SupervisionState::Ready;
                return SpawnOutcome {
                    state: SupervisionState::Ready,
                    listen: Some(base.clone()),
                    message: "external mode connected".to_string(),
                };
            }
            *self.state.lock().await = SupervisionState::Failed;
            return SpawnOutcome {
                state: SupervisionState::Failed,
                listen: None,
                message: format!("external service unreachable: {base}"),
            };
        }

        *self.state.lock().await = SupervisionState::Starting;
        // 孤儿清理：runtime.json 里有 PID 且进程存活 → 先尝试关闭。
        self.cleanup_orphan().await;

        let Some(listen) = Self::pick_port(&self.config.listen).await else {
            *self.state.lock().await = SupervisionState::Failed;
            return SpawnOutcome {
                state: SupervisionState::Failed,
                listen: None,
                message: "no free port in 17621..17640".to_string(),
            };
        };

        let config_path = self.config.data_dir.join("config.toml");
        let db_path = self.config.data_dir.join("token-router-v2.db");
        let child = tokio::process::Command::new(&self.config.sidecar_path)
            .args([
                "--config",
                &config_path.to_string_lossy(),
                "--db",
                &db_path.to_string_lossy(),
                "--listen",
                &listen,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                *self.state.lock().await = SupervisionState::Failed;
                return SpawnOutcome {
                    state: SupervisionState::Failed,
                    listen: None,
                    message: format!(
                        "spawn sidecar at {}: {error}",
                        self.config.sidecar_path.display()
                    ),
                };
            }
        };

        // stderr 尾部（诊断用）：非阻塞读一次；就绪探测期间进程可能输出。
        let mut stderr_tail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            use tokio::io::AsyncReadExt;
            let mut buffer = [0u8; 4096];
            if let Ok(Ok(n)) =
                tokio::time::timeout(Duration::from_millis(100), stderr.read(&mut buffer)).await
            {
                stderr_tail.push_str(&String::from_utf8_lossy(&buffer[..n]));
            }
            child.stderr = Some(stderr);
        }

        let base = format!("http://{listen}");
        if Self::wait_ready(&base, Duration::from_secs(15)).await {
            *self.child.lock().await = Some(child);
            *self.listen_addr.lock().await = Some(listen.clone());
            *self.state.lock().await = SupervisionState::Ready;
            self.write_runtime_json(&listen).await;
            SpawnOutcome {
                state: SupervisionState::Ready,
                listen: Some(listen),
                message: String::new(),
            }
        } else {
            let _ = child.kill().await;
            *self.state.lock().await = SupervisionState::Failed;
            SpawnOutcome {
                state: SupervisionState::Failed,
                listen: None,
                message: format!(
                    "sidecar not ready on {listen} (path: {}){}",
                    self.config.sidecar_path.display(),
                    if stderr_tail.is_empty() {
                        String::new()
                    } else {
                        format!("; stderr: {stderr_tail}")
                    }
                ),
            }
        }
    }

    /// 带退避的重启：1s/2s/4s/8s/16s，最多 5 次。
    pub async fn restart_with_backoff(&self) -> SpawnOutcome {
        *self.state.lock().await = SupervisionState::Restarting;
        let mut backoff = 1u64;
        for _ in 0..5 {
            let outcome = self.start().await;
            if outcome.state == SupervisionState::Ready {
                *self.restarts.lock().await += 1;
                return outcome;
            }
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff *= 2;
        }
        *self.state.lock().await = SupervisionState::Failed;
        SpawnOutcome {
            state: SupervisionState::Failed,
            listen: None,
            message: "restart failed after 5 attempts (see logs)".to_string(),
        }
    }

    /// 停止：`POST /v2/admin/shutdown` → 等待 5s → kill。
    pub async fn stop(&self) -> String {
        let listen = self.listen_addr().await;
        if let Some(listen) = &listen {
            let _ = reqwest_post(&format!("{listen}/v2/admin/shutdown"), &serde_json::json!({})).await;
            // 最多等 5s。
            for _ in 0..50 {
                let exited = {
                    let mut guard = self.child.lock().await;
                    match guard.as_mut() {
                        Some(child) => child.try_wait().ok().flatten().is_some(),
                        None => true,
                    }
                };
                if exited {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
        let message = format!("stopped ({listen:?})");
        *self.state.lock().await = SupervisionState::Stopped;
        *self.listen_addr.lock().await = None;
        self.remove_runtime_json().await;
        message
    }

    /// 孤儿清理：读 runtime.json，PID 存活但 /health 不通 → 尝试关闭。
    async fn cleanup_orphan(&self) {
        let runtime_path = self.config.data_dir.join("runtime.json");
        let Ok(text) = std::fs::read_to_string(&runtime_path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            let _ = std::fs::remove_file(&runtime_path);
            return;
        };
        let Some(pid) = value["pid"].as_u64() else {
            let _ = std::fs::remove_file(&runtime_path);
            return;
        };
        // 进程存活检测（Windows：tasklist；Unix：kill 0）。
        let alive = process_alive(pid);
        let healthy = match value["listen"].as_str() {
            Some(listen) => health_ok(&format!("http://{listen}")).await,
            None => false,
        };
        if alive && !healthy {
            // PID 活着但服务不健康：孤儿候选，强杀。
            kill_process(pid);
        }
        let _ = std::fs::remove_file(&runtime_path);
    }

    async fn write_runtime_json(&self, listen: &str) {
        let pid = {
            let guard = self.child.lock().await;
            guard.as_ref().and_then(|child| child.id()).unwrap_or(0)
        };
        let value = serde_json::json!({
            "pid": pid,
            "listen": listen,
            "started_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        });
        let path = self.config.data_dir.join("runtime.json");
        let temp = self.config.data_dir.join("runtime.json.tmp");
        if std::fs::write(&temp, value.to_string()).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }

    async fn remove_runtime_json(&self) {
        let _ = std::fs::remove_file(self.config.data_dir.join("runtime.json"));
    }
}

/// 就绪/健康探测与 shutdown POST：直接用 reqwest（workspace 依赖）。
async fn reqwest_get(url: &str) -> Result<reqwest::Response, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?
        .get(url)
        .send()
        .await
}

async fn reqwest_post(url: &str, body: &serde_json::Value) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    client
        .post(url)
        .json(body)
        .send()
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

async fn health_ok(base: &str) -> bool {
    matches!(
        reqwest_get(&format!("{base}/health")).await,
        Ok(response) if response.status().is_success()
    )
}

fn process_alive(pid: u64) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

fn kill_process(pid: u64) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output();
    }
}
