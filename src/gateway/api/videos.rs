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
        has_last_frame = req.last_frame.is_some(),
        reference_images = req.reference_images.len(),
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

pub async fn videos_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video_id): Path<String>,
) -> AppResult<Json<VideoObject>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let job = state.videos.store().require(&video_id)?;
    let config = state.config();
    let target = config.resolve_video_upstream(&job.tier);
    let obj = state.videos.cancel(target.as_ref(), &video_id).await?;
    Ok(Json(obj))
}

pub async fn videos_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(video_id): Path<String>,
) -> AppResult<StatusCode> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let job = state.videos.store().require(&video_id)?;
    let config = state.config();
    let target = config.resolve_video_upstream(&job.tier);
    state.videos.delete(target.as_ref(), &video_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn parse_create_json(bytes: &[u8]) -> AppResult<VideoCreateRequest> {
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
    let resolution = v
        .get("resolution")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

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

    let last_frame = v
        .get("last_frame_url")
        .and_then(|u| u.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|u| ImageRef::Url(u.to_string()));

    let mut reference_images = Vec::new();
    if let Some(arr) = v.get("reference_image_urls").and_then(|a| a.as_array()) {
        for item in arr {
            if let Some(url) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                reference_images.push(ImageRef::Url(url.to_string()));
            }
        }
    }

    let watermark = match v.get("watermark") {
        Some(Value::Bool(true)) => Some(true),
        Some(Value::Bool(false)) => Some(false),
        _ => None,
    };

    Ok(VideoCreateRequest {
        prompt,
        model,
        seconds,
        size,
        input_reference,
        resolution,
        last_frame,
        reference_images,
        watermark,
    })
}

async fn parse_create_multipart(mut multipart: Multipart) -> AppResult<VideoCreateRequest> {
    let mut prompt: Option<String> = None;
    let mut model: Option<String> = None;
    let mut seconds: Option<String> = None;
    let mut size: Option<String> = None;
    let mut resolution: Option<String> = None;
    let mut input_reference: Option<ImageRef> = None;
    let mut last_frame: Option<ImageRef> = None;
    let mut reference_images: Vec<ImageRef> = Vec::new();
    let mut watermark: Option<bool> = None;

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
            "resolution" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("resolution: {e}")))?;
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    resolution = Some(trimmed.to_string());
                }
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
            "last_frame_url" => {
                let url = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("last_frame_url: {e}")))?;
                let trimmed = url.trim();
                if !trimmed.is_empty() {
                    last_frame = Some(ImageRef::Url(trimmed.to_string()));
                }
            }
            "reference_image_urls" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("reference_image_urls: {e}")))?;
                // Accept JSON array or newline/comma-separated URLs.
                if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&text) {
                    for item in arr {
                        if let Some(url) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            reference_images.push(ImageRef::Url(url.to_string()));
                        }
                    }
                } else {
                    for part in text.split([',', '\n']) {
                        let url = part.trim();
                        if !url.is_empty() {
                            reference_images.push(ImageRef::Url(url.to_string()));
                        }
                    }
                }
            }
            "watermark" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("watermark: {e}")))?;
                match text.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => watermark = Some(true),
                    "false" | "0" | "no" => watermark = Some(false),
                    _ => {}
                }
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
        resolution,
        last_frame,
        reference_images,
        watermark,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_herdsman_h3_body() {
        let body = r#"{
            "prompt": "a cat walks on the beach",
            "model": "MiniMax-H3",
            "seconds": "5",
            "size": "1280x720",
            "image_url": "data:image/png;base64,AAAA",
            "resolution": "768P",
            "last_frame_url": "https://example.com/last.png",
            "reference_image_urls": [],
            "watermark": true
        }"#;
        let req = parse_create_json(body.as_bytes()).unwrap();
        assert_eq!(req.prompt, "a cat walks on the beach");
        assert_eq!(req.model.as_deref(), Some("MiniMax-H3"));
        assert_eq!(req.seconds.as_deref(), Some("5"));
        assert_eq!(req.size.as_deref(), Some("1280x720"));
        assert_eq!(req.resolution.as_deref(), Some("768P"));
        assert!(matches!(req.input_reference, Some(ImageRef::Url(_))));
        assert!(matches!(req.last_frame, Some(ImageRef::Url(ref u)) if u.contains("last.png")));
        assert!(req.reference_images.is_empty());
        assert_eq!(req.watermark, Some(true));
    }

    #[test]
    fn parse_reference_images() {
        let body = r#"{
            "prompt": "cat",
            "reference_image_urls": ["https://a/1.png", "https://a/2.png"]
        }"#;
        let req = parse_create_json(body.as_bytes()).unwrap();
        assert_eq!(req.reference_images.len(), 2);
        assert!(req.input_reference.is_none());
        assert!(req.last_frame.is_none());
    }

    #[test]
    fn parse_ignores_false_watermark_as_false() {
        let body = r#"{"prompt":"x","watermark":false}"#;
        let req = parse_create_json(body.as_bytes()).unwrap();
        assert_eq!(req.watermark, Some(false));
    }
}
