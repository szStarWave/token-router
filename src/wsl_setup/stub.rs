use serde::Serialize;

use crate::agent_setup::AgentSetupResult;

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

pub fn detect() -> Result<WslDetectResult, String> {
    Ok(WslDetectResult {
        available: false,
        running_distros: Vec::new(),
        message: Some("WSL is only available on Windows".to_string()),
    })
}

pub fn configure_agent(
    _distro: &str,
    _agent: &str,
    _api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    Err("WSL agent configure is only available on Windows".to_string())
}

pub fn wsl_agents() -> &'static [&'static str] {
    &[]
}
