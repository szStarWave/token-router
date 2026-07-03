pub mod auth_keys;
pub mod file;
pub mod paths;
pub mod setup;

pub use auth_keys::{
    collect_inbound_api_keys, create_gateway_auth_key, delete_gateway_auth_key,
    list_gateway_auth_keys, migrate_legacy_gateway_api_key, update_gateway_auth_key_name,
    CreateGatewayAuthKeyRequest, CreateGatewayAuthKeyResponse, GatewayAuthKeyView,
    UpdateGatewayAuthKeyRequest,
};
pub use file::{
    ConfigFile, GatewayApiKeyEntry, UpstreamEndpoint, ensure_initialized, load, load_from_path,
    save,
};
pub use setup::{
    GatewayConfigPatch, GatewayConfigView, UpstreamEndpointPatch, UpstreamEndpointView,
    UpstreamSetupUpdate, UpstreamSetupView, apply_default_upstream, apply_setup_patch,
    apply_upstream_patch, endpoint_configured, gateway_view_from_section, is_setup_validation_error,
    normalize_client_http_url, view_from_config, CLOUD_MODEL_AUTO, DEFAULT_CLOUD_BUDGET_AGENT_ID,
    mask_gateway_api_key,
};
pub use paths::{
    app_dir, callme_file, config_file, display_app_dir, display_home, ensure_app_dirs,
    gateway_log_file, logs_dir, pid_file, sessions_dir, stats_db, stats_file, user_home,
};
