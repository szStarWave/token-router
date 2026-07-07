use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use token_router::config::setup::listen_port_from_addr;
use token_router::gateway::AppConfig;

use crate::agent_setup::{
    agent_deploy_state_at, agent_init_status_at, configure_claude_code_at, configure_codex_at,
    configure_hermes_at, configure_openclaw_at, configure_opencode_at, AgentInitStatus,
    AgentSetupResult,
};

const WSL_AGENTS: &[&str] = &["openclaw", "hermes", "claude-code", "codex", "opencode"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslAgentDetectItem {
    pub agent: String,
    pub initialized: bool,
    pub deployed: bool,
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslDistroInfo {
    pub name: String,
    pub home_path: Option<String>,
    pub gateway_host: Option<String>,
    pub gateway_v1_base: Option<String>,
    pub gateway_anthropic_base: Option<String>,
    pub gateway_verified: bool,
    pub agents: Vec<WslAgentDetectItem>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslDetectResult {
    pub available: bool,
    pub running_distros: Vec<WslDistroInfo>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslConfigureResult {
    pub distro: String,
    pub gateway_host: String,
    pub configured: Vec<AgentSetupResult>,
    pub skipped: Vec<String>,
}

fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if bytes.contains(&0) {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk.get(1).copied().unwrap_or(0)]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn run_wsl_raw(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("wsl.exe")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run wsl.exe: {e}"))
}

fn run_wsl(distro: &str, script: &str) -> Result<String, String> {
    // Use non-login shell so user .bashrc/.profile (e.g. project venv hooks) is not sourced.
    let output = run_wsl_raw(&["-d", distro, "--", "sh", "-c", script])?;
    if !output.status.success() {
        let stderr = decode_wsl_output(&output.stderr);
        let stdout = decode_wsl_output(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(if detail.is_empty() {
            "wsl command failed".to_string()
        } else {
            detail
        });
    }
    Ok(decode_wsl_output(&output.stdout).trim().to_string())
}

fn unix_path_to_unc(distro: &str, unix_path: &str) -> Result<PathBuf, String> {
    let unix_path = unix_path.trim();
    if unix_path.is_empty() || !unix_path.starts_with('/') {
        return Err(format!("invalid WSL path: {unix_path}"));
    }
    let rel = unix_path.trim_start_matches('/').replace('/', "\\");
    for prefix in ["\\\\wsl.localhost\\", "\\\\wsl$\\"] {
        let path = PathBuf::from(format!("{prefix}{distro}\\{rel}"));
        if path.is_dir() {
            return Ok(path);
        }
    }
    Err(format!(
        "WSL path not accessible for distro `{distro}` ({unix_path})"
    ))
}

fn wsl_home_unc(distro: &str) -> Result<PathBuf, String> {
    let home = run_wsl(
        distro,
        "if [ -n \"$HOME\" ]; then printf '%s' \"$HOME\"; \
else H=$(getent passwd \"$(id -un)\" 2>/dev/null | cut -d: -f6); \
[ -n \"$H\" ] && printf '%s' \"$H\"; fi",
    )?;
    if home.is_empty() {
        return Err(format!("could not resolve home directory for WSL distro `{distro}`"));
    }
    unix_path_to_unc(distro, &home)
}

fn parse_wsl_verbose_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim().trim_start_matches('*').trim();
    if trimmed.is_empty() || trimmed.starts_with("NAME") {
        return None;
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let version = parts.last()?.to_string();
    if version != "1" && version != "2" {
        return None;
    }
    let state = parts[parts.len() - 2].to_string();
    let name = parts[..parts.len() - 2].join(" ");
    if name.is_empty() {
        return None;
    }
    Some((name, state))
}

fn list_running_wsl_distros() -> Result<Vec<String>, String> {
    let output = run_wsl_raw(&["--list", "--verbose"])?;
    if !output.status.success() {
        return Err(decode_wsl_output(&output.stderr).trim().to_string());
    }
    Ok(decode_wsl_output(&output.stdout)
        .lines()
        .filter_map(parse_wsl_verbose_line)
        .filter(|(_, state)| state.eq_ignore_ascii_case("Running"))
        .map(|(name, _)| name)
        .collect())
}

fn wsl_available() -> bool {
    match run_wsl_raw(&["--status"]) {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

fn wsl_can_reach(distro: &str, host: &str, port: u16) -> bool {
    let tcp_script = format!(
        "command -v bash >/dev/null 2>&1 && bash -c \"exec 3<>/dev/tcp/{host}/{port}\" 2>/dev/null"
    );
    if run_wsl(distro, &tcp_script).is_ok() {
        return true;
    }
    let http_script = format!(
        "H='{host}'; P={port}; \
if command -v curl >/dev/null 2>&1; then curl -sf --max-time 3 \"http://$H:$P/health\" >/dev/null 2>&1; \
elif command -v wget >/dev/null 2>&1; then wget -q --timeout=3 -O /dev/null \"http://$H:$P/health\" 2>/dev/null; \
else exit 1; fi"
    );
    run_wsl(distro, &http_script).is_ok()
}

fn wsl_host_ip(distro: &str) -> Result<String, String> {
    let ip = run_wsl(
        distro,
        "awk '/^nameserver/{print $2; exit}' /etc/resolv.conf",
    )?;
    if ip.is_empty() {
        return Err("failed to resolve Windows host IP from WSL".to_string());
    }
    Ok(ip)
}

fn wsl_gateway_host_candidates(distro: &str) -> Vec<String> {
    let mut hosts = vec!["127.0.0.1".to_string(), "localhost".to_string()];
    if let Ok(ip) = run_wsl(
        distro,
        "ip -4 route show default 2>/dev/null | awk '{print $3; exit}'",
    ) {
        let ip = ip.trim().to_string();
        if !ip.is_empty() && !hosts.iter().any(|h| h == &ip) {
            hosts.push(ip);
        }
    }
    if let Ok(ip) = wsl_host_ip(distro) {
        if !hosts.iter().any(|h| h == &ip) {
            hosts.push(ip);
        }
    }
    hosts
}

struct WslGatewayHost {
    host: String,
    verified: bool,
    warning: Option<String>,
}

fn pick_wsl_gateway_host(distro: &str, port: u16) -> WslGatewayHost {
    let candidates = wsl_gateway_host_candidates(distro);
    for host in &candidates {
        if wsl_can_reach(distro, host, port) {
            return WslGatewayHost {
                host: host.clone(),
                verified: true,
                warning: None,
            };
        }
    }
    let fallback = candidates
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let tried = candidates.join(", ");
    let warning = format!(
        "Token Router gateway could not be verified from WSL (tried {tried}:{port}); using {fallback} as fallback"
    );
    WslGatewayHost {
        host: fallback,
        verified: false,
        warning: Some(warning),
    }
}

fn gateway_urls_from_host(host: &str, port: u16) -> (String, String, String) {
    let base = format!("http://{host}:{port}");
    (
        base.clone(),
        format!("{base}/v1"),
        format!("{base}/anthropic"),
    )
}

fn detect_agents(home: &Path) -> Vec<WslAgentDetectItem> {
    let mut items = Vec::new();
    for agent in WSL_AGENTS {
        let init = agent_init_status_at(home, agent).unwrap_or_else(|_| AgentInitStatus {
            agent: (*agent).to_string(),
            initialized: false,
            config_path: String::new(),
        });
        let deployed = agent_deploy_state_at(home, agent)
            .map(|s| s.deployed)
            .unwrap_or(false);
        items.push(WslAgentDetectItem {
            agent: (*agent).to_string(),
            initialized: init.initialized,
            deployed,
            config_path: init.config_path,
        });
    }
    items
}

fn detect_distro(distro: &str, port: u16) -> WslDistroInfo {
    let home_result = wsl_home_unc(distro);
    let home_path = home_result.as_ref().ok().map(|p| p.display().to_string());
    let agents = home_result
        .as_ref()
        .map(|home| detect_agents(home))
        .unwrap_or_default();

    match home_result {
        Ok(_) => {
            let gateway = pick_wsl_gateway_host(distro, port);
            let (_base, v1, anthropic) = gateway_urls_from_host(&gateway.host, port);
            WslDistroInfo {
                name: distro.to_string(),
                home_path,
                gateway_host: Some(gateway.host),
                gateway_v1_base: Some(v1),
                gateway_anthropic_base: Some(anthropic),
                gateway_verified: gateway.verified,
                agents,
                message: gateway.warning,
            }
        }
        Err(message) => WslDistroInfo {
            name: distro.to_string(),
            home_path: None,
            gateway_host: None,
            gateway_v1_base: None,
            gateway_anthropic_base: None,
            gateway_verified: false,
            agents,
            message: Some(message),
        },
    }
}

fn detect_wsl_environment() -> Result<WslDetectResult, String> {
    if !wsl_available() {
        return Ok(WslDetectResult {
            available: false,
            running_distros: Vec::new(),
            message: Some("WSL is not installed or not running".to_string()),
        });
    }

    let running = list_running_wsl_distros()?;
    if running.is_empty() {
        return Ok(WslDetectResult {
            available: true,
            running_distros: Vec::new(),
            message: Some("No running WSL distro found".to_string()),
        });
    }

    let config = AppConfig::load().map_err(|e| e.to_string())?;
    let port = listen_port_from_addr(&config.listen_addr);
    let running_distros = running
        .iter()
        .map(|distro| detect_distro(distro, port))
        .collect();

    Ok(WslDetectResult {
        available: true,
        running_distros,
        message: None,
    })
}

fn ensure_running_distro(distro: &str) -> Result<(), String> {
    let running = list_running_wsl_distros()?;
    if running
        .iter()
        .any(|name| name.eq_ignore_ascii_case(distro))
    {
        return Ok(());
    }
    Err(format!("WSL distro `{distro}` is not running"))
}

fn configure_wsl_agents(distro: &str, api_key: Option<String>) -> Result<WslConfigureResult, String> {
    ensure_running_distro(distro)?;
    let home = wsl_home_unc(distro)?;

    let config = AppConfig::load().map_err(|e| e.to_string())?;
    let port = listen_port_from_addr(&config.listen_addr);
    let gateway = pick_wsl_gateway_host(distro, port);
    let (_base, openai_v1_base, anthropic_base) = gateway_urls_from_host(&gateway.host, port);

    let mut configured = Vec::new();
    let mut skipped = Vec::new();

    for agent in WSL_AGENTS {
        let init = agent_init_status_at(&home, agent)?;
        if !init.initialized {
            skipped.push(format!("{agent}: not initialized"));
            continue;
        }
        let result = match *agent {
            "openclaw" => configure_openclaw_at(&home, &openai_v1_base, api_key.clone()),
            "hermes" => configure_hermes_at(&home, agent, &openai_v1_base, api_key.clone()),
            "claude-code" => configure_claude_code_at(&home, &anthropic_base, api_key.clone()),
            "codex" => configure_codex_at(&home, &openai_v1_base, api_key.clone()),
            "opencode" => configure_opencode_at(&home, &openai_v1_base, api_key.clone()),
            _ => continue,
        };
        match result {
            Ok(item) => configured.push(item),
            Err(err) => skipped.push(format!("{agent}: {err}")),
        }
    }

    if configured.is_empty() && !skipped.is_empty() {
        return Err(skipped.join("; "));
    }

    Ok(WslConfigureResult {
        distro: distro.to_string(),
        gateway_host: gateway.host,
        configured,
        skipped,
    })
}

#[tauri::command]
pub fn wsl_detect_environment() -> Result<WslDetectResult, String> {
    detect_wsl_environment()
}

#[tauri::command]
pub fn wsl_configure_agents(
    distro: String,
    api_key: Option<String>,
) -> Result<WslConfigureResult, String> {
    let distro = distro.trim();
    if distro.is_empty() {
        return Err("WSL distro is required".to_string());
    }
    configure_wsl_agents(distro, api_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_wsl_output_handles_utf16() {
        let bytes: Vec<u8> = "Ubuntu\0".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert!(decode_wsl_output(&bytes).contains("Ubuntu"));
    }

    #[test]
    fn gateway_urls_from_host_builds_expected_paths() {
        let (base, v1, anthropic) = gateway_urls_from_host("127.0.0.1", 11080);
        assert_eq!(base, "http://127.0.0.1:11080");
        assert_eq!(v1, "http://127.0.0.1:11080/v1");
        assert_eq!(anthropic, "http://127.0.0.1:11080/anthropic");
    }

    #[test]
    fn parse_wsl_verbose_line_extracts_running_distro() {
        let (name, state) =
            parse_wsl_verbose_line("* Ubuntu-22.04           Running         2").unwrap();
        assert_eq!(name, "Ubuntu-22.04");
        assert_eq!(state, "Running");
    }

    #[test]
    fn unix_path_to_unc_includes_distro_name() {
        let path = unix_path_to_unc("Debian", "/home/user").unwrap_err();
        assert!(path.contains("Debian"));
        assert!(path.contains("/home/user"));
    }
}
