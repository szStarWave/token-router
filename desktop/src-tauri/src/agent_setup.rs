//! Thin Tauri wrappers over `token_router::agent_setup`.

pub use token_router::agent_setup::{
    AgentDeployStatus, AgentInitStatus, AgentSetupResult, ERR_AGENT_NOT_INITIALIZED,
    agent_deploy_state_at, agent_init_status_at, configure_claude_code_at, configure_codebuddy_at,
    configure_codex_at, configure_hermes_at, configure_openclaw_at, configure_opencode_at,
    configure_workbuddy_at,
};

use token_router::agent_setup;

#[tauri::command]
pub fn check_agent_initialized(agent: String) -> Result<AgentInitStatus, String> {
    agent_setup::check_agent_initialized(agent)
}

#[tauri::command]
pub fn check_agent_deployed(agent: String) -> Result<AgentDeployStatus, String> {
    agent_setup::check_agent_deployed(agent)
}

#[tauri::command]
pub fn configure_openclaw_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    agent_setup::configure_openclaw_agent(api_key)
}

#[tauri::command]
pub fn configure_hermes_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    agent_setup::configure_hermes_agent(api_key)
}

#[tauri::command]
pub fn configure_hermes_flash_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    agent_setup::configure_hermes_flash_agent(api_key)
}

#[tauri::command]
pub fn configure_claude_code_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    agent_setup::configure_claude_code_agent(api_key, context_window)
}

#[tauri::command]
pub fn configure_codex_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    agent_setup::configure_codex_agent(api_key, context_window)
}

#[tauri::command]
pub fn configure_opencode_agent(api_key: Option<String>) -> Result<AgentSetupResult, String> {
    agent_setup::configure_opencode_agent(api_key)
}

#[tauri::command]
pub fn configure_codebuddy_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    agent_setup::configure_codebuddy_agent(api_key, context_window)
}

#[tauri::command]
pub fn configure_workbuddy_agent(
    api_key: Option<String>,
    context_window: Option<u64>,
) -> Result<AgentSetupResult, String> {
    agent_setup::configure_workbuddy_agent(api_key, context_window)
}

#[tauri::command]
pub fn read_inbound_auth_key_cmd(preferred_name: Option<String>) -> Result<Option<String>, String> {
    agent_setup::read_inbound_auth_key_cmd(preferred_name)
}

#[tauri::command]
pub fn read_default_auth_key_cmd() -> Result<Option<String>, String> {
    agent_setup::read_default_auth_key_cmd()
}
