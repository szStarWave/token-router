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
    ConfigFile, GatewayApiKeyEntry, ImageUpstreamEndpoint, ImageUpstreamSection, UpstreamEndpoint,
    VideoUpstreamEndpoint, VideoUpstreamSection, apply_port_override, ensure_initialized, load,
    load_from_path, save,
};
pub use setup::{
    GatewayConfigPatch, GatewayConfigView, ImageUpstreamEndpointPatch, ImageUpstreamEndpointView,
    UpstreamEndpointPatch, UpstreamEndpointView, UpstreamSetupUpdate, UpstreamSetupView,
    VideoUpstreamEndpointPatch, VideoUpstreamEndpointView, apply_default_upstream,
    apply_setup_patch, apply_upstream_patch, endpoint_configured, gateway_view_from_section,
    image_endpoint_configured, is_setup_validation_error, normalize_client_http_url,
    video_endpoint_configured, view_from_config, CLOUD_MODEL_AUTO, DEFAULT_CLOUD_BUDGET_AGENT_ID,
    mask_gateway_api_key,
};
pub use paths::{
    app_dir, callme_file, callme_file_at, config_file, display_app_dir, display_home,
    ensure_app_dirs, gateway_log_file, gateway_log_file_at, logs_dir, logs_dir_at, pid_file,
    pid_file_at, resolve_app_dir, resolve_config_file, sessions_dir, sessions_dir_at, stats_db,
    stats_db_at, stats_file, stats_file_at, user_home,
};
