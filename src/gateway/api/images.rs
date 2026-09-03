use axum::{
    Json,
    extract::{Multipart, State},
    http::HeaderMap,
};
use tracing::info;

use crate::gateway::api::auth::require_gateway_api_key;
use crate::gateway::api::routes::{extract_agent_id, AppState};
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::image::tier::resolve_image_tier;
use crate::gateway::image::types::{
    ImageBytes, ImageEditRequest, ImageGenerateRequest, ImagesResponse,
};

pub async fn images_generations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImageGenerateRequest>,
) -> AppResult<Json<ImagesResponse>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    if req.prompt.trim().is_empty() {
        return Err(AppError::BadRequest("prompt is required".into()));
    }

    let config = state.config();
    let (tier, reasons) = resolve_image_tier(&config, Some(state.edge_load.as_ref()))?;
    let target = config
        .resolve_image_upstream(tier.as_str())
        .ok_or_else(|| AppError::Unavailable(format!("image {} upstream missing", tier.as_str())))?;

    info!(
        tier = tier.as_str(),
        provider = %target.provider,
        reasons = ?reasons,
        "images.generations"
    );

    let resp = state.images.generate(&target, tier, &req).await?;
    Ok(Json(resp))
}

pub async fn images_edits(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> AppResult<Json<ImagesResponse>> {
    require_gateway_api_key(&headers, &state.config().inbound_api_keys)?;
    let _agent_id = extract_agent_id(&headers);

    let edit = parse_edit_multipart(multipart).await?;
    if edit.prompt.trim().is_empty() {
        return Err(AppError::BadRequest("prompt is required".into()));
    }

    let config = state.config();
    let (tier, reasons) = resolve_image_tier(&config, Some(state.edge_load.as_ref()))?;
    let target = config
        .resolve_image_upstream(tier.as_str())
        .ok_or_else(|| AppError::Unavailable(format!("image {} upstream missing", tier.as_str())))?;

    info!(
        tier = tier.as_str(),
        provider = %target.provider,
        reasons = ?reasons,
        "images.edits"
    );

    let resp = state.images.edit(&target, tier, &edit).await?;
    Ok(Json(resp))
}

async fn parse_edit_multipart(mut multipart: Multipart) -> AppResult<ImageEditRequest> {
    let mut prompt: Option<String> = None;
    let mut model: Option<String> = None;
    let mut n: Option<u32> = None;
    let mut size: Option<String> = None;
    let mut response_format: Option<String> = None;
    let mut user: Option<String> = None;
    let mut image: Option<ImageBytes> = None;
    let mut mask: Option<ImageBytes> = None;

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
            "n" => {
                let t = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("n: {e}")))?;
                n = t.trim().parse().ok();
            }
            "size" => {
                size = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("size: {e}")))?,
                );
            }
            "response_format" => {
                response_format = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("response_format: {e}")))?,
                );
            }
            "user" => {
                user = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("user: {e}")))?,
                );
            }
            "image" | "image[]" => {
                let filename = field
                    .file_name()
                    .unwrap_or("image.png")
                    .to_string();
                let mime = field
                    .content_type()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "image/png".to_string());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("image: {e}")))?
                    .to_vec();
                if bytes.is_empty() {
                    return Err(AppError::BadRequest("image is empty".into()));
                }
                // First image wins in v1.
                if image.is_none() {
                    image = Some(ImageBytes {
                        bytes,
                        mime,
                        filename,
                    });
                }
            }
            "mask" => {
                let filename = field.file_name().unwrap_or("mask.png").to_string();
                let mime = field
                    .content_type()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "image/png".to_string());
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("mask: {e}")))?
                    .to_vec();
                if !bytes.is_empty() {
                    mask = Some(ImageBytes {
                        bytes,
                        mime,
                        filename,
                    });
                }
            }
            _ => {
                // Drain unknown fields.
                let _ = field.bytes().await;
            }
        }
    }

    let image = image.ok_or_else(|| AppError::BadRequest("image is required".into()))?;
    let prompt = prompt.ok_or_else(|| AppError::BadRequest("prompt is required".into()))?;

    Ok(ImageEditRequest {
        model,
        prompt,
        image,
        mask,
        n,
        size,
        response_format,
        user,
    })
}
