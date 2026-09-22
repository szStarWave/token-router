use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use bytes::Bytes;

use crate::gateway::api::auth::require_gateway_api_key;
use crate::gateway::api::routes::{extract_agent_id, AppState};
use crate::gateway::config::AppConfig;
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::tier::{resolve_video_tier, VideoTier};
use crate::gateway::video::{VideoForwardRequest, parse_forward_method};

fn resolve_requested_tier(
    headers: &HeaderMap,
    config: &AppConfig,
    edge_load: Option<&EdgeInferenceTracker>,
) -> AppResult<(VideoTier, Vec<String>)> {
    let requested = headers
        .get("x-video-tier")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());
    match requested.as_deref() {
        Some("edge") if config.video_edge.is_some() => {
            Ok((VideoTier::Edge, vec!["CLIENT_VIDEO_TIER_EDGE".into()]))
        }
        Some("cloud") if config.video_cloud.is_some() => {
            Ok((VideoTier::Cloud, vec!["CLIENT_VIDEO_TIER_CLOUD".into()]))
        }
        Some("edge") => Err(AppError::Unavailable(
            "X-Video-Tier=edge but video edge upstream is not configured".into(),
        )),
        Some("cloud") => Err(AppError::Unavailable(
            "X-Video-Tier=cloud but video cloud upstream is not configured".into(),
        )),
        Some(other) => Err(AppError::BadRequest(format!(
            "X-Video-Tier must be edge or cloud, got `{other}`"
        ))),
        None => resolve_video_tier(config, edge_load),
    }
}

pub async fn video_forward(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> AppResult<Response> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let method = headers
        .get("x-video-method")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("X-Video-Method is required".into()))?;
    let method = parse_forward_method(method)?;
    let path = headers
        .get("x-video-path")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("X-Video-Path is required".into()))?
        .to_string();

    let body = axum::body::to_bytes(request.into_body(), crate::gateway::video::forward::MAX_FORWARD_BODY)
        .await
        .map_err(|e| AppError::BadRequest(format!("read body: {e}")))?;

    let config = state.config();
    let (tier, _reasons) = resolve_requested_tier(&headers, &config, Some(state.edge_load.as_ref()))?;
    let target = config
        .resolve_video_upstream(tier.as_str())
        .ok_or_else(|| AppError::Unavailable(format!("video {} upstream missing", tier.as_str())))?;

    let forwarded = state
        .videos
        .forward(
            &target,
            tier,
            &VideoForwardRequest {
                method,
                path,
                headers,
                body: Bytes::from(body),
            },
        )
        .await?;

    let mut response = Response::new(Body::from(forwarded.body));
    *response.status_mut() = forwarded.status;
    if let Some(content_type) = forwarded.content_type.as_deref() {
        if let Ok(value) = HeaderValue::from_str(content_type) {
            response.headers_mut().insert(header::CONTENT_TYPE, value);
        }
    }
    Ok(response)
}
