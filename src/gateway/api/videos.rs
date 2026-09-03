use axum::{
    Json,
    extract::{FromRequest, Multipart, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use serde_json::Value;
use tracing::info;

use crate::gateway::api::auth::require_gateway_api_key;
use crate::gateway::api::routes::{extract_agent_id, AppState};
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::tier::resolve_video_tier;
use crate::gateway::video::types::{
    ImageRef, VideoCreateRequest, VideoListResponse, VideoObject,
};

#[derive(Debug, Deserialize)]
pub struct VideoListQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub after: Option<String>,
    #[serde(default = "default_order")]
    pub order: String,
}

fn default_limit() -> u32 {
    20
}

fn default_order() -> String {
    "desc".into()
}

#[derive(Debug, Deserialize)]
pub struct VideoContentQuery {
    pub variant: Option<String>,
}

pub async fn videos_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> AppResult<Json<VideoObject>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let req = if content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        let multipart = Multipart::from_request(request, &state)
            .await
            .map_err(|e| AppError::BadRequest(format!("multipart: {e}")))?;
        parse_create_multipart(multipart).await?
    } else {
        let bytes = axum::body::to_bytes(request.into_body(), 32 * 1024 * 1024)
            .await
            .map_err(|e| AppError::BadRequest(format!("read body: {e}")))?;
        parse_create_json(&bytes)?
    };

    if req.prompt.trim().is_empty() {
        return Err(AppError::BadRequest("prompt is required".into()));
    }

    let config = state.config();
    let (tier, reasons) = resolve_video_tier(&config, Some(state.edge_load.as_ref()))?;
    let target = config
        .resolve_video_upstream(tier.as_str())
        .ok_or_else(|| AppError::Unavailable(format!("video {} upstream missing", tier.as_str())))?;

    info!(
        tier = tier.as_str(),
        provider = %target.provider,
        reasons = ?reasons,
        has_reference = req.input_reference.is_some(),
        "videos.create"
    );

    let obj = state.videos.create(&target, tier, &req).await?;
    Ok(Json(obj))
}

pub async fn videos_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<VideoListQuery>,
) -> AppResult<Json<VideoListResponse>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let limit = q.limit.clamp(1, 100) as usize;
    let order_desc = !q.order.eq_ignore_ascii_case("asc");
    let resp = state.videos.list(limit, q.after.as_deref(), order_desc)?;
    Ok(Json(resp))
}

pub async fn videos_retrieve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video_id): Path<String>,
) -> AppResult<Json<VideoObject>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let job = state.videos.store().require(&video_id)?;
    let config = state.config();
    let target = config.resolve_video_upstream(&job.tier);
    let obj = state.videos.retrieve(target.as_ref(), &video_id).await?;
    Ok(Json(obj))
}

pub async fn videos_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video_id): Path<String>,
    Query(q): Query<VideoContentQuery>,
) -> AppResult<Response> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let job = state.videos.store().require(&video_id)?;
    let config = state.config();
    let target = config.resolve_video_upstream(&job.tier);
    let (bytes, mime) = state
        .videos
        .download_content(target.as_ref(), &video_id, q.variant.as_deref())
        .await?;

    let mut response = Response::new(axum::body::Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(mime),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"video.mp4\""),
    );
    Ok(response)
}

fn parse_create_json(bytes: &[u8]) -> AppResult<VideoCreateRequest> {
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|e| AppError::BadRequest(format!("invalid json: {e}")))?;
    let prompt = v
        .get("prompt")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let model = v
        .get("model")
        .and_then(|m| m.as_str())
        .map(str::to_string);
    let seconds = match v.get("seconds") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let size = v.get("size").and_then(|s| s.as_str()).map(str::to_string);

    let input_reference = if let Some(obj) = v.get("input_reference") {
        if let Some(url) = obj.get("image_url").and_then(|u| u.as_str()) {
            Some(ImageRef::Url(url.to_string()))
        } else if let Some(url) = obj.as_str() {
            Some(ImageRef::Url(url.to_string()))
        } else {
            None
        }
    } else if let Some(url) = v.get("image_url").and_then(|u| u.as_str()) {
        Some(ImageRef::Url(url.to_string()))
    } else {
        None
    };

    Ok(VideoCreateRequest {
        prompt,
        model,
        seconds,
        size,
        input_reference,
    })
}

async fn parse_create_multipart(mut multipart: Multipart) -> AppResult<VideoCreateRequest> {
    let mut prompt: Option<String> = None;
    let mut model: Option<String> = None;
    let mut seconds: Option<String> = None;
    let mut size: Option<String> = None;
    let mut input_reference: Option<ImageRef> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "prompt" => {
                prompt = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("prompt: {e}")))?,
                );
            }
            "model" => {
                model = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("model: {e}")))?,
                );
            }
            "seconds" => {
                seconds = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("seconds: {e}")))?,
                );
            }
            "size" => {
                size = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("size: {e}")))?,
                );
            }
            "input_reference" | "image" => {
                let filename = field
                    .file_name()
                    .unwrap_or("input_reference.png")
                    .to_string();
                let mime = field
                    .content_type()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| guess_mime(&filename));
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("input_reference: {e}")))?;
                input_reference = Some(ImageRef::Bytes {
                    bytes: bytes.to_vec(),
                    mime,
                    filename,
                });
            }
            "image_url" => {
                let url = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("image_url: {e}")))?;
                input_reference = Some(ImageRef::Url(url));
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    Ok(VideoCreateRequest {
        prompt: prompt.unwrap_or_default(),
        model,
        seconds,
        size,
        input_reference,
    })
}

fn guess_mime(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png".into()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".into()
    } else if lower.ends_with(".webp") {
        "image/webp".into()
    } else {
        "application/octet-stream".into()
    }
}
