//! Thin Tauri wrappers over `token_router::wsl_setup`.

pub use token_router::wsl_setup::{
    WslAgentDetectItem, WslDetectResult, WslDistroInfo,
};

use token_router::agent_setup::AgentSetupResult;
use token_router::wsl_setup;

#[tauri::command]
pub async fn wsl_detect_environment() -> Result<WslDetectResult, String> {
    tauri::async_runtime::spawn_blocking(wsl_setup::detect)
        .await
        .map_err(|err| format!("WSL detection task failed: {err}"))?
}

#[tauri::command]
pub async fn wsl_configure_agent(
    distro: String,
    agent: String,
    api_key: Option<String>,
) -> Result<AgentSetupResult, String> {
    let distro = distro.trim().to_string();
    let agent = agent.trim().to_string();
    if distro.is_empty() {
        return Err("WSL distro is required".to_string());
    }
    if agent.is_empty() {
        return Err("Agent is required".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || wsl_setup::configure_agent(&distro, &agent, api_key))
        .await
        .map_err(|err| format!("WSL configure task failed: {err}"))?
}
