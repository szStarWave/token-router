//! Agent 接入 host 命令（技术方案 §8）：detect → plan → apply → verify。
//!
//! 安全：
//! - token 明文只由 host 写入 Agent 配置；plan 预览里的 credential 用引用占位；
//! - setup 请求只带 token 引用（env:/file:），host 在 apply 时解析成真实值；
//! - WSL 发行版名白名单校验；wsl.exe 参数数组调用。

use tr_onboard::{AgentKind, SetupRequest, Target};

use crate::AppState;

/// detect 输入：agent + 可选 WSL 发行版。
#[derive(Debug, serde::Deserialize)]
pub struct OnboardDetectArgs {
    pub agent: String,
    /// WSL 发行版名；空/缺省 = 宿主。
    #[serde(default)]
    pub wsl_distro: Option<String>,
}

/// setup 请求（UI → host）。token 以引用传递。
#[derive(Debug, serde::Deserialize)]
pub struct OnboardSetupArgs {
    pub agent: String,
    #[serde(default)]
    pub wsl_distro: Option<String>,
    /// 网关 base URL（外部模式或自定义端口时由 UI 传入）。
    #[serde(default)]
    pub gateway_url: Option<String>,
    /// 凭据引用：`env:NAME` / `file:path`。
    pub credential_ref: String,
    /// 要写入的模型（虚拟模型 id）。
    #[serde(default = "default_model")]
    pub model: String,
    /// 显式切换入口：true 时覆盖客户端已有的 model 键；Onboarding 默认 false。
    #[serde(default)]
    pub force_model: bool,
}

fn default_model() -> String {
    "auto".to_string()
}

fn parse_agent(id: &str) -> Result<AgentKind, String> {
    AgentKind::parse(id).ok_or_else(|| format!("unknown agent `{id}`"))
}

fn parse_target(distro: &Option<String>) -> Result<Target, String> {
    match distro.as_deref() {
        Some(name) if !name.is_empty() => {
            tr_onboard::wsl::target(name).map_err(|error| error.to_string())
        }
        _ => Ok(Target::Host),
    }
}

/// detect：安装/配置状态。
#[tauri::command]
pub async fn onboard_detect(
    _state: tauri::State<'_, AppState>,
    args: OnboardDetectArgs,
) -> Result<tr_onboard::DetectReport, String> {
    let agent = parse_agent(&args.agent)?;
    let target = parse_target(&args.wsl_distro)?;
    tauri::async_runtime::spawn_blocking(move || {
        tr_onboard::detect(&target, agent).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// plan：合并后全文预览（不写盘；credential 用引用占位）。
#[tauri::command]
pub async fn onboard_plan(
    _state: tauri::State<'_, AppState>,
    args: OnboardSetupArgs,
) -> Result<tr_onboard::SetupPlan, String> {
    let agent = parse_agent(&args.agent)?;
    let target = parse_target(&args.wsl_distro)?;
    let request = SetupRequest {
        gateway_url: args
            .gateway_url
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:17621".to_string()),
        credential_ref: args.credential_ref.clone(),
        model: args.model.clone(),
        force_model: args.force_model,
    };
    tauri::async_runtime::spawn_blocking(move || {
        tr_onboard::plan(&target, agent, &request).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// apply：备份 + 原子写。写入时把引用解析为真实 token（仅 env:/file:）。
#[tauri::command]
pub async fn onboard_apply(
    state: tauri::State<'_, AppState>,
    args: OnboardSetupArgs,
) -> Result<tr_onboard::ApplyReport, String> {
    let agent = parse_agent(&args.agent)?;
    let target = parse_target(&args.wsl_distro)?;
    let credential = resolve_credential(&args.credential_ref)?;
    // WSL 场景：网关地址要用宿主在 WSL 里的可达 IP。
    let gateway_url = match (&args.wsl_distro, &args.gateway_url) {
        (Some(distro), None) if !distro.is_empty() => {
            let listen = state.supervisor.listen_addr().await;
            let port = listen
                .as_deref()
                .and_then(|listen| listen.rsplit(':').next())
                .and_then(|port| port.parse::<u16>().ok())
                .unwrap_or(17621);
            tr_onboard::wsl::probe_gateway_url(distro, port).map_err(|error| error.to_string())?
        }
        (_, Some(url)) => url.clone(),
        _ => format!(
            "http://{}",
            state
                .supervisor
                .listen_addr()
                .await
                .unwrap_or_else(|| "127.0.0.1:17621".to_string())
        ),
    };
    let request = SetupRequest {
        gateway_url,
        credential_ref: credential,
        model: args.model.clone(),
        force_model: args.force_model,
    };
    // 先 plan（解析后的真实值产出 after 全文），再 apply。
    tauri::async_runtime::spawn_blocking(move || {
        let plan = tr_onboard::plan(&target, agent, &request).map_err(|error| error.to_string())?;
        tr_onboard::apply(&target, &plan).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// verify：读回配置校验。
#[tauri::command]
pub async fn onboard_verify(
    _state: tauri::State<'_, AppState>,
    args: OnboardDetectArgs,
) -> Result<tr_onboard::VerifyReport, String> {
    let agent = parse_agent(&args.agent)?;
    let target = parse_target(&args.wsl_distro)?;
    tauri::async_runtime::spawn_blocking(move || {
        tr_onboard::verify(&target, agent).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// WSL 环境检测。
#[tauri::command]
pub async fn onboard_wsl_detect() -> Result<tr_onboard::wsl::WslReport, String> {
    tauri::async_runtime::spawn_blocking(|| Ok(tr_onboard::wsl::detect()))
        .await
        .map_err(|error| error.to_string())?
}

/// 解析凭据引用：`env:NAME` / `file:path`；其他一律拒绝（密钥不进配置字面量）。
fn resolve_credential(reference: &str) -> Result<String, String> {
    if let Some(name) = reference.strip_prefix("env:") {
        return std::env::var(name)
            .map_err(|_| format!("env `{name}` 未设置（先在系统环境变量中配置）"));
    }
    if let Some(path) = reference.strip_prefix("file:") {
        return std::fs::read_to_string(path)
            .map(|text| text.trim().to_string())
            .map_err(|error| format!("读取 {path}: {error}"));
    }
    Err("credential_ref 只支持 env:NAME 或 file:path；明文密钥不落配置".to_string())
}
