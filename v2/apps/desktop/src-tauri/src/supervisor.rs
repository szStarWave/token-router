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
        let default_port = listen
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(17621);
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

    /// 等指定监听地址真正可绑定（重启时旧进程退出后端口释放有延迟）。
    async fn wait_listen_free(listen: &str, timeout: Duration) -> bool {
        let Ok(addr) = listen.parse::<SocketAddr>() else {
            return true;
        };
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // 绑上即空（listener 立即 drop，仅作探测）。
            if tokio::net::TcpListener::bind(addr).await.is_ok() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 外部服务地址（`TR_EXTERNAL_BASE_URL`）；None 表示由本壳拉起 sidecar。
    pub fn external_base(&self) -> Option<&str> {
        self.config.external.as_deref()
    }

    /// 是否连接外部服务（不托管进程）。前端以此隐藏本地启停动作。
    pub fn is_external(&self) -> bool {
        self.config.external.is_some()
    }

    /// 启动（或连接外部）服务端；就绪后写 `runtime.json`。
    pub async fn start(&self) -> SpawnOutcome {
        // 外部模式：只探测可达性。
        if let Some(base) = &self.config.external {
            return self.connect_external(base, Duration::from_secs(15)).await;
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
        let mut command = tokio::process::Command::new(&self.config.sidecar_path);
        command
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
            .kill_on_drop(true);
        // sidecar 是 console 子系统程序：release 壳（GUI 子系统）拉起它时不要弹控制台窗口。
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let child = command.spawn();

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

    /// 连接外部服务：只探测 `/health` 可达性，不 spawn、不写 runtime.json。
    /// `timeout` 可注入（生产 15s，测试毫秒级）。
    async fn connect_external(&self, base: &str, timeout: Duration) -> SpawnOutcome {
        *self.state.lock().await = SupervisionState::Starting;
        if Self::wait_ready(base, timeout).await {
            *self.listen_addr.lock().await = Some(base.to_string());
            *self.state.lock().await = SupervisionState::Ready;
            return SpawnOutcome {
                state: SupervisionState::Ready,
                listen: Some(base.to_string()),
                message: "external mode connected".to_string(),
            };
        }
        *self.state.lock().await = SupervisionState::Failed;
        SpawnOutcome {
            state: SupervisionState::Failed,
            listen: None,
            message: format!("external service unreachable: {base}"),
        }
    }

    /// 带退避的重启：1s/2s/4s/8s/16s，最多 5 次。
    ///
    /// 外部模式：远端进程不归本壳托管，重启无从谈起——
    /// 直接返回明确错误（前端据此隐藏重启动作），不做假重启。
    pub async fn restart_with_backoff(&self) -> SpawnOutcome {
        if let Some(base) = &self.config.external {
            return SpawnOutcome {
                state: self.state().await,
                listen: self.listen_addr().await,
                message: format!(
                    "restart unavailable in external mode: service at {base} is not managed by this shell"
                ),
            };
        }
        // 先停旧的再拉起：旧进程还占着端口时 `start()` 会顺延到下一个端口
        // （17621 → 17622），配置里的 listen 与实际监听就分叉了——按固定
        // base-url 连的客户端（tr-cli、外部集成）会连不上。
        let _ = self.stop().await;
        // 旧进程退出后端口可能短暂仍不可绑（TIME_WAIT / 句柄释放），等它真正空出来。
        Self::wait_listen_free(&self.config.listen, Duration::from_secs(5)).await;

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
    ///
    /// 外部模式（`external` 为 `Some`）：服务进程不归本壳托管，
    /// 绝不向远端发 shutdown——只断开本地连接（清 base URL / 状态归 Stopped）。
    pub async fn stop(&self) -> String {
        let listen = self.listen_addr().await;
        if self.config.external.is_none() {
            if let Some(listen) = &listen {
                // listen 存的是 `127.0.0.1:port`，POST 需要 http:// 前缀（reqwest 要绝对 URL）。
                let base = crate::base_url_of(Some(listen));
                if let Some(base) = base {
                    let _ =
                        reqwest_post(&format!("{base}/v2/admin/shutdown"), &serde_json::json!({}))
                            .await;
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
            }
            if let Some(mut child) = self.child.lock().await.take() {
                let _ = child.kill().await;
            }
        }
        let message = match &self.config.external {
            Some(_) => format!("disconnected from external service ({listen:?})"),
            None => format!("stopped ({listen:?})"),
        };
        *self.state.lock().await = SupervisionState::Stopped;
        *self.listen_addr.lock().await = None;
        // runtime.json 记录的是本壳拉起的 sidecar；外部模式没写过它，
        // 也不能删（可能是之前本地托管遗留的孤儿追踪记录）。
        if self.config.external.is_none() {
            self.remove_runtime_json().await;
        }
        message
    }

    /// 孤儿清理：读 runtime.json。记录的是本壳上一次托管的 sidecar；本壳当前没有
    /// 存活子进程时，只要 PID 还在就强杀——健康与否都算孤儿（壳崩溃/被强杀后
    /// sidecar 会活下来占住端口，留着会逼新实例换端口）。
    async fn cleanup_orphan(&self) {
        // 本壳已有存活子进程：runtime.json 指向的就是它，不动。
        if self.child.lock().await.is_some() {
            return;
        }
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
        if process_alive(pid) {
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
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 外部模式配置：`base` 由测试内临时 TCP 监听器提供。
    fn external_config(base: String, data_dir: &std::path::Path) -> SupervisorConfig {
        SupervisorConfig {
            sidecar_path: PathBuf::from("token-router-v2"),
            data_dir: data_dir.to_path_buf(),
            listen: "127.0.0.1:17621".to_string(),
            external: Some(base),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tr-supervisor-ext-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// 极简 HTTP 服务器：逐请求记录 path，统一回 200（body 随意）。
    /// 用于断言「外部模式下 stop 不向远端发 shutdown」。
    struct RecordingServer {
        addr: std::net::SocketAddr,
        paths: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingServer {
        async fn spawn() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind :0");
            let addr = listener.local_addr().expect("local_addr");
            let paths: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let worker_paths = paths.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    let paths = worker_paths.clone();
                    tokio::spawn(async move {
                        use tokio::io::AsyncReadExt;
                        let mut buffer = vec![0u8; 4096];
                        // 读到请求头结束（或读满/对端关）即算收到请求。
                        let mut received = Vec::new();
                        while let Ok(n) = socket.read(&mut buffer).await {
                            if n == 0 {
                                break;
                            }
                            received.extend_from_slice(&buffer[..n]);
                            if received.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        let text = String::from_utf8_lossy(&received);
                        let path = text
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or_default()
                            .to_string();
                        if !path.is_empty() {
                            paths.lock().await.push(path);
                        }
                        let _ = socket
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                            .await;
                        let _ = socket.shutdown().await;
                    });
                }
            });
            Self { addr, paths }
        }

        fn base(&self) -> String {
            format!("http://{}", self.addr)
        }

        async fn requested_paths(&self) -> Vec<String> {
            self.paths.lock().await.clone()
        }
    }

    use std::sync::Arc;
    use tokio::io::AsyncWriteExt;

    /// external = Some 时 is_external() 为 true（前端据此隐藏本地启停）。
    #[tokio::test]
    async fn is_external_reflects_config() {
        let data_dir = temp_dir("flag");
        let base = "http://127.0.0.1:1".to_string(); // 不会真正连接
        let supervisor = Supervisor::new(external_config(base, &data_dir));
        assert!(supervisor.is_external());

        let supervisor = Supervisor::new(SupervisorConfig {
            external: None,
            ..external_config(String::new(), &data_dir)
        });
        assert!(!supervisor.is_external());
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// 重启前的端口等待：占用时返回 false，释放后返回 true。
    #[tokio::test]
    async fn wait_listen_free_tracks_port_release() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind :0");
        let addr = listener.local_addr().expect("local_addr").to_string();
        assert!(
            !Supervisor::wait_listen_free(&addr, Duration::from_millis(200)).await,
            "被占用的端口不应判为空闲"
        );
        drop(listener);
        assert!(
            Supervisor::wait_listen_free(&addr, Duration::from_secs(2)).await,
            "释放后端口应可绑定"
        );
    }

    /// 外部模式 start：/health 可达 → Ready，listen 即 base；不 spawn、不写 runtime.json。
    #[tokio::test]
    async fn external_start_connects_when_health_ok() {
        let data_dir = temp_dir("start-ok");
        let server = RecordingServer::spawn().await;
        let supervisor = Supervisor::new(external_config(server.base(), &data_dir));

        let outcome = supervisor.start().await;
        assert_eq!(
            outcome.state,
            SupervisionState::Ready,
            "{}",
            outcome.message
        );
        assert_eq!(outcome.listen.as_deref(), Some(server.base().as_str()));
        assert_eq!(supervisor.state().await, SupervisionState::Ready);
        assert!(
            !data_dir.join("runtime.json").exists(),
            "外部模式不写 runtime.json"
        );

        let _ = supervisor.stop().await;
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// 外部模式 start：/health 不可达 → Failed（显式 unreachable 消息）。
    #[tokio::test]
    async fn external_start_fails_when_unreachable() {
        let data_dir = temp_dir("start-fail");
        // 预留一个空闲端口再释放：大概率没人监听。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind :0");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);

        let supervisor = Supervisor::new(external_config(format!("http://{addr}"), &data_dir));
        // 走 connect_external（start 的外部分支委托），注入短超时避免 15s 探测。
        let outcome = supervisor
            .connect_external(&format!("http://{addr}"), Duration::from_millis(600))
            .await;
        assert_eq!(outcome.state, SupervisionState::Failed);
        assert!(
            outcome.message.contains("unreachable"),
            "{}",
            outcome.message
        );
        assert_eq!(supervisor.listen_addr().await, None);
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// 关键安全边界：外部模式下 stop 绝不向远端 POST /v2/admin/shutdown，
    /// 只断开本地连接（状态归 Stopped、listen 清空），远端保持 Ready。
    #[tokio::test]
    async fn external_stop_never_posts_shutdown() {
        let data_dir = temp_dir("stop");
        let server = RecordingServer::spawn().await;
        let supervisor = Supervisor::new(external_config(server.base(), &data_dir));

        let outcome = supervisor.start().await;
        assert_eq!(
            outcome.state,
            SupervisionState::Ready,
            "{}",
            outcome.message
        );
        server.requested_paths().await.clear();

        let message = supervisor.stop().await;
        assert_eq!(supervisor.state().await, SupervisionState::Stopped);
        assert_eq!(supervisor.listen_addr().await, None, "本地连接已断开");
        assert!(message.contains("disconnected"), "{}", message);

        // 远端只收到过 start 探测的 /health，绝无 shutdown。
        let paths = server.requested_paths().await;
        assert!(
            !paths.iter().any(|p| p.contains("shutdown")),
            "external stop 不得向远端发 shutdown，实际请求: {paths:?}"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// 外部模式 restart：立即返回明确错误，不改状态、不进退避循环（不做假重启）。
    #[tokio::test]
    async fn external_restart_returns_explicit_error() {
        let data_dir = temp_dir("restart");
        let server = RecordingServer::spawn().await;
        let supervisor = Supervisor::new(external_config(server.base(), &data_dir));

        let outcome = supervisor.start().await;
        assert_eq!(
            outcome.state,
            SupervisionState::Ready,
            "{}",
            outcome.message
        );

        let outcome = supervisor.restart_with_backoff().await;
        assert_eq!(
            outcome.state,
            SupervisionState::Ready,
            "状态不被 restart 破坏"
        );
        assert_eq!(outcome.listen.as_deref(), Some(server.base().as_str()));
        assert!(
            outcome
                .message
                .contains("restart unavailable in external mode"),
            "{}",
            outcome.message
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
