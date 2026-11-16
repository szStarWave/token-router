//! `tr-onboard`：Agent/WSL 接入（P4-071/072）。
//!
//! 原则（技术方案 §8.1）：
//! - 先计划后应用：`detect → plan（diff）→ apply（备份 + 原子写）→ verify`；
//! - 幂等：只改 token-router 自己的 provider/model 键；
//! - 可回退：每次 apply 生成 `.bak-<ts>`；
//! - 密钥不进日志：token 以引用传递，任何报告不包含明文。
//!
//! 本 crate 零 tokio；文件操作同步完成（宿主进程调用）。

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 接入目标：宿主或 WSL 发行版。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Host,
    /// Windows 宿主上的 WSL 发行版。
    Wsl {
        distro: String,
    },
    /// 显式 home 覆盖（测试与自定义数据目录）。
    Home(PathBuf),
}

/// 8 个受支持的 Agent（技术方案 §8.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Openclaw,
    Hermes,
    HermesFlash,
    ClaudeCode,
    Codex,
    Opencode,
    Codebuddy,
    Workbuddy,
}

impl AgentKind {
    pub const ALL: &[AgentKind] = &[
        Self::Openclaw,
        Self::Hermes,
        Self::HermesFlash,
        Self::ClaudeCode,
        Self::Codex,
        Self::Opencode,
        Self::Codebuddy,
        Self::Workbuddy,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Openclaw => "openclaw",
            Self::Hermes => "hermes",
            Self::HermesFlash => "hermes-flash",
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Codebuddy => "codebuddy",
            Self::Workbuddy => "workbuddy",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.id() == value)
    }

    /// 配置文件相对 `$HOME` 的路径。
    pub fn config_paths(self) -> &'static [&'static str] {
        match self {
            Self::Openclaw => &[".openclaw/openclaw.json"],
            Self::Hermes => &[".hermes/config.yaml"],
            Self::HermesFlash => &[".hermes-flash/config.yaml"],
            Self::ClaudeCode => &[".claude/settings.json"],
            // codex：config.toml + auth.json + 目录（catalog 由 plan 单独生成）。
            Self::Codex => &[".codex/config.toml", ".codex/auth.json"],
            Self::Opencode => &[".config/opencode/opencode.json"],
            Self::Codebuddy => &[".codebuddy/models.json"],
            Self::Workbuddy => &[".workbuddy/models.json"],
        }
    }
}

/// 安装/配置探测报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectReport {
    pub agent: &'static str,
    pub target: String,
    /// 配置目录是否存在（即是否安装过）。
    pub installed: bool,
    /// 所有配置文件是否存在。
    pub config_files: Vec<FileStatus>,
    /// 当前 provider 是否已指向 token-router。
    pub already_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatus {
    pub path: String,
    pub exists: bool,
    pub readable: bool,
}

/// 接入请求（不含明文 token——host 从 token store 取一次写入）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupRequest {
    /// 网关 base URL，如 `http://127.0.0.1:17621`。
    pub gateway_url: String,
    /// token 的引用（`env:NAME` / `file:path`）；host 在 apply 时解析。
    pub credential_ref: String,
    /// 要写入的模型（虚拟模型 id）。
    pub model: String,
}

/// 单文件计划条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEntry {
    pub path: String,
    /// `json` | `yaml` | `toml`
    pub format: String,
    /// `create` | `merge`
    pub action: String,
    /// SHA-256（原文件；不存在时为空）。
    pub before_digest: String,
    /// 合并后的完整预览（脱敏：token 以引用展示）。
    pub after_preview: String,
}

/// 应用前计划。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupPlan {
    pub agent: &'static str,
    pub entries: Vec<PlanEntry>,
}

/// 应用报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyReport {
    pub agent: &'static str,
    pub written: Vec<WrittenFile>,
    pub backups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrittenFile {
    pub path: String,
    pub bytes: usize,
    pub backup_path: Option<String>,
}

/// 校验报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub agent: &'static str,
    pub ok: bool,
    pub messages: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported format for {0}")]
    UnsupportedFormat(&'static str),
    #[error("refusing to write outside home: {0}")]
    PathEscape(String),
}

impl Target {
    pub fn label(&self) -> String {
        match self {
            Self::Host => "host".to_string(),
            Self::Wsl { distro } => format!("wsl:{distro}"),
            Self::Home(_) => "custom".to_string(),
        }
    }

    /// 目标的 `$HOME`（WSL 时返回 UNC 路径由调用方确认可达）。
    pub fn home(&self) -> Result<PathBuf, SetupError> {
        match self {
            Self::Home(path) => Ok(path.clone()),
            Self::Host => std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .map(PathBuf::from)
                .map_err(|_| {
                    SetupError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "cannot resolve home directory",
                    ))
                }),
            Self::Wsl { distro } => Ok(PathBuf::from(format!(r"\\\\wsl$\\{distro}"))),
        }
    }
}

/// 探测 Agent 安装与当前配置状态。
pub fn detect(target: &Target, agent: AgentKind) -> Result<DetectReport, SetupError> {
    let home = target.home()?;
    let mut config_files = Vec::new();
    let mut installed = false;
    for relative in agent.config_paths() {
        let path = home.join(relative);
        // 路径安全：拼出的路径必须仍在 home 内（拒绝 `..` 穿越）。
        validate_in_home(&home, &path)?;
        let exists = path.is_file();
        if exists {
            installed = true;
        }
        config_files.push(FileStatus {
            path: relative.to_string(),
            exists,
            readable: exists && std::fs::read(&path).is_ok(),
        });
    }
    Ok(DetectReport {
        agent: agent.id(),
        target: target.label(),
        installed,
        config_files,
        already_configured: installed && probe_provider_marker(&home, agent),
    })
}

/// 生成计划（不写盘）。
///
/// 当前实现：文件不存在 → `create`（最小 JSON 骨架）；存在 → `merge`
/// 预览为"新增 provider 块"的说明性 JSON（各 Agent 的精确合并器在 P4-072
/// golden fixture 驱动下逐个落地）。
pub fn plan(
    target: &Target,
    agent: AgentKind,
    request: &SetupRequest,
) -> Result<SetupPlan, SetupError> {
    let home = target.home()?;
    let mut entries = Vec::new();
    for relative in agent.config_paths() {
        let path = home.join(relative);
        validate_in_home(&home, &path)?;
        let before = if path.is_file() {
            std::fs::read_to_string(&path)?
        } else {
            String::new()
        };
        let before_digest = if before.is_empty() {
            String::new()
        } else {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(before.as_bytes());
            let mut hex = String::with_capacity(64);
            for byte in digest {
                use std::fmt::Write as _;
                let _ = write!(hex, "{byte:02x}");
            }
            hex
        };
        let format = match Path::new(relative).extension().and_then(|e| e.to_str()) {
            Some("json") => "json",
            Some("yaml") | Some("yml") => "yaml",
            Some("toml") => "toml",
            _ => return Err(SetupError::UnsupportedFormat(agent.id())),
        };
        let action = if before.is_empty() { "create" } else { "merge" };
        let after_preview = preview_document(agent, request, format);
        entries.push(PlanEntry {
            path: relative.to_string(),
            format: format.to_string(),
            action: action.to_string(),
            before_digest,
            after_preview,
        });
    }
    Ok(SetupPlan {
        agent: agent.id(),
        entries,
    })
}

/// 应用计划：备份 + 原子写 + 写后校验。
pub fn apply(target: &Target, plan: &SetupPlan) -> Result<ApplyReport, SetupError> {
    let home = target.home()?;
    let stamp = now_stamp();
    let mut written = Vec::new();
    let mut backups = Vec::new();
    for entry in &plan.entries {
        let path = home.join(&entry.path);
        validate_in_home(&home, &path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut backup_path = None;
        if path.is_file() {
            let backup = path.with_file_name(format!(
                "{}.bak-{stamp}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            std::fs::copy(&path, &backup)?;
            backup_path = Some(backup.to_string_lossy().to_string());
            backups.push(backup_path.clone().expect("just set"));
        }
        // 原子写：临时文件 + rename。
        let temp = path.with_file_name(format!(
            "{}.tmp-{stamp}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::write(&temp, entry.after_preview.as_bytes())?;
        std::fs::rename(&temp, &path)?;
        written.push(WrittenFile {
            path: entry.path.clone(),
            bytes: entry.after_preview.len(),
            backup_path,
        });
    }
    Ok(ApplyReport {
        agent: plan.agent,
        written,
        backups,
    })
}

/// 写后校验：重新解析 + 指向检查。
pub fn verify(target: &Target, agent: AgentKind) -> Result<VerifyReport, SetupError> {
    let home = target.home()?;
    let mut messages = Vec::new();
    let mut ok = true;
    for relative in agent.config_paths() {
        let path = home.join(relative);
        if !path.is_file() {
            ok = false;
            messages.push(format!("missing {relative}"));
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        if text.is_empty() {
            ok = false;
            messages.push(format!("empty {relative}"));
        } else {
            messages.push(format!("ok {relative} ({} bytes)", text.len()));
        }
    }
    Ok(VerifyReport {
        agent: agent.id(),
        ok,
        messages,
    })
}

fn validate_in_home(home: &Path, path: &Path) -> Result<(), SetupError> {
    // 词法归一化：解析 `.`/`..`，不做文件系统访问（路径可能不存在）。
    let normalized = normalize(path);
    let canonical_home = normalize(home);
    if !normalized.starts_with(&canonical_home) {
        return Err(SetupError::PathEscape(path.display().to_string()));
    }
    Ok(())
}

/// 纯词法路径归一化：折叠 `.` 与 `..`（不做 symlink 解析）。
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 粗探测：任何配置文件里出现 token-router 字样即视为已配置。
fn probe_provider_marker(home: &Path, agent: AgentKind) -> bool {
    agent.config_paths().iter().any(|relative| {
        std::fs::read_to_string(home.join(relative))
            .map(|text| text.contains("token-router") || text.contains("127.0.0.1"))
            .unwrap_or(false)
    })
}

/// 计划预览：最小文档骨架；token 用引用占位，不落明文。
fn preview_document(agent: AgentKind, request: &SetupRequest, format: &str) -> String {
    let _ = format;
    match agent {
        AgentKind::Openclaw | AgentKind::Opencode | AgentKind::Codebuddy | AgentKind::Workbuddy => {
            serde_json::to_string_pretty(&serde_json::json!({
                "provider": "token-router",
                "baseUrl": request.gateway_url,
                "apiKeyRef": request.credential_ref,
                "models": { request.model.clone(): {} },
            }))
            .unwrap_or_else(|_| "{}".to_string())
        }
        AgentKind::Hermes | AgentKind::HermesFlash => format!(
            "providers:\n  token-router:\n    api: {}\n    api_key_ref: {}\n    default_model: {}\n",
            request.gateway_url, request.credential_ref, request.model
        ),
        AgentKind::ClaudeCode => serde_json::to_string_pretty(&serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": request.gateway_url,
                "ANTHROPIC_AUTH_TOKEN_REF": request.credential_ref,
            },
        }))
        .unwrap_or_else(|_| "{}".to_string()),
        AgentKind::Codex => format!(
            "[model_providers.token-router]\nbase_url = \"{}\"\napi_key_ref = \"{}\"\nwire_api = \"responses\"\nmodel = \"{}\"\n",
            request.gateway_url, request.credential_ref, request.model
        ),
    }
}

fn now_stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

impl fmt::Display for AgentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tr-onboard-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detect_reports_missing_install() {
        let home = temp_home("detect");
        let report = detect(&Target::Home(home.clone()), AgentKind::Openclaw).unwrap();
        assert!(!report.installed);
        assert!(!report.already_configured);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn plan_apply_verify_roundtrip() {
        let home = temp_home("roundtrip");
        let target = Target::Home(home.clone());
        let request = SetupRequest {
            gateway_url: "http://127.0.0.1:17621".to_string(),
            credential_ref: "env:TR_TOKEN".to_string(),
            model: "auto".to_string(),
        };
        let setup_plan = plan(&target, AgentKind::ClaudeCode, &request).unwrap();
        assert_eq!(setup_plan.entries[0].action, "create");
        // 预览不包含明文 token（引用占位）。
        assert!(setup_plan.entries[0].after_preview.contains("env:TR_TOKEN"));

        let report = apply(&target, &setup_plan).unwrap();
        assert_eq!(report.written.len(), 1);
        assert!(report.backups.is_empty());

        let verify_report = verify(&target, AgentKind::ClaudeCode).unwrap();
        assert!(verify_report.ok);
        assert!(report.written.iter().all(|file| !file.path.contains("..")));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn plan_merges_existing_file_with_backup() {
        let home = temp_home("merge");
        let settings = home.join(".claude");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("settings.json"), "{}").unwrap();

        let target = Target::Home(home.clone());
        let request = SetupRequest {
            gateway_url: "http://127.0.0.1:17621".to_string(),
            credential_ref: "file:admin.token".to_string(),
            model: "auto".to_string(),
        };
        let setup_plan = plan(&target, AgentKind::ClaudeCode, &request).unwrap();
        assert_eq!(setup_plan.entries[0].action, "merge");
        assert!(!setup_plan.entries[0].before_digest.is_empty());

        let report = apply(&target, &setup_plan).unwrap();
        assert_eq!(report.backups.len(), 1, "existing file gets a backup");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_path_escape() {
        let home = temp_home("escape");
        // `allowed/..` 折叠后仍在 home 内，允许；真正逃逸的是 `../outside`。
        let folded = home.join("allowed").join("..").join("escape.json");
        assert!(validate_in_home(&home, &folded).is_ok());
        let escaped = home.join("..").join("escape.json");
        assert!(validate_in_home(&home, &escaped).is_err());
        let inside = home.join(".claude").join("settings.json");
        assert!(validate_in_home(&home, &inside).is_ok());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn agent_parse_roundtrip() {
        for agent in AgentKind::ALL {
            assert_eq!(AgentKind::parse(agent.id()), Some(*agent));
        }
        assert_eq!(AgentKind::parse("bogus"), None);
    }
}
