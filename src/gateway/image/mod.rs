//! Image generation / edit providers and routing.

mod comfyui;
mod dashscope;
mod openai;
mod seedream;
pub mod tier;
pub mod types;

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;

use crate::gateway::config::ResolvedImageUpstream;
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::error::{AppError, AppResult};

use self::types::{ImageEditRequest, ImageGenerateRequest, ImagesResponse};

pub use tier::{resolve_image_tier, ImageTier};

const IMAGE_HTTP_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone)]
pub struct ImageClient {
    http: Client,
    edge_load: Arc<EdgeInferenceTracker>,
}

impl ImageClient {
    pub fn new(edge_load: Arc<EdgeInferenceTracker>) -> Self {
        let http = Client::builder()
            .timeout(IMAGE_HTTP_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { http, edge_load }
    }

    pub async fn generate(
        &self,
        target: &ResolvedImageUpstream,
        tier: ImageTier,
        req: &ImageGenerateRequest,
    ) -> AppResult<ImagesResponse> {
        let _guard = if tier == ImageTier::Edge {
            Some(self.edge_load.begin())
        } else {
            None
        };
        dispatch_generate(&self.http, target, req).await
    }

    pub async fn edit(
        &self,
        target: &ResolvedImageUpstream,
        tier: ImageTier,
        req: &ImageEditRequest,
    ) -> AppResult<ImagesResponse> {
        let _guard = if tier == ImageTier::Edge {
            Some(self.edge_load.begin())
        } else {
            None
        };
        dispatch_edit(&self.http, target, req).await
    }
}

async fn dispatch_generate(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
) -> AppResult<ImagesResponse> {
    match target.provider.as_str() {
        "openai" => openai::generate(http, target, req).await,
        "seedream" => seedream::generate(http, target, req).await,
        "dashscope" => dashscope::generate(http, target, req).await,
        "comfyui" => comfyui::generate(http, target, req).await,
        other => Err(AppError::BadRequest(format!(
            "unsupported image provider `{other}`"
        ))),
    }
}

async fn dispatch_edit(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageEditRequest,
) -> AppResult<ImagesResponse> {
    match target.provider.as_str() {
        "openai" => openai::edit(http, target, req).await,
        "seedream" => seedream::edit(http, target, req).await,
        "dashscope" => dashscope::edit(http, target, req).await,
        "comfyui" => comfyui::edit(http, target, req).await,
        other => Err(AppError::BadRequest(format!(
            "unsupported image provider `{other}`"
        ))),
    }
}

pub(crate) fn resolve_model_name(
    target: &ResolvedImageUpstream,
    request_model: Option<&str>,
) -> Option<String> {
    use crate::gateway::api::codex_catalog::is_router_auto_model;

    let prefer = target
        .upstream_model
        .as_deref()
        .or(target.model.as_deref())
        .map(str::trim)
        .filter(|m| !m.is_empty() && !is_router_auto_model(m));
    if prefer.is_some() {
        return prefer.map(str::to_string);
    }
    request_model
        .map(str::trim)
        .filter(|m| !m.is_empty() && !is_router_auto_model(m))
        .map(str::to_string)
}

pub(crate) fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

pub(crate) fn openai_error_message(status: reqwest::StatusCode, body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = v
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
        {
            return format!("upstream {status}: {msg}");
        }
        if let Some(msg) = v
            .get("message")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
        {
            return format!("upstream {status}: {msg}");
        }
    }
    let snippet: String = body.chars().take(300).collect();
    if snippet.is_empty() {
        format!("upstream HTTP {status}")
    } else {
        format!("upstream HTTP {status}: {snippet}")
    }
}

pub(crate) async fn maybe_download_b64(
    http: &Client,
    resp: ImagesResponse,
    want_b64: bool,
) -> AppResult<ImagesResponse> {
    if !want_b64 {
        return Ok(resp);
    }
    let mut out = resp;
    for item in &mut out.data {
        if item.b64_json.is_some() {
            continue;
        }
        let Some(url) = item.url.clone() else {
            continue;
        };
        let bytes = http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("download image: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Upstream(format!("download image: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Upstream(format!("download image body: {e}")))?;
        item.b64_json = Some(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &bytes,
        ));
        item.url = None;
    }
    Ok(out)
}

pub(crate) fn size_to_dashscope(size: Option<&str>) -> String {
    let s = size.unwrap_or("1024x1024");
    s.replace('x', "*").replace('X', "*")
}

pub(crate) fn parse_wxh(size: Option<&str>) -> (u32, u32) {
    let s = size.unwrap_or("1024x1024");
    let parts: Vec<&str> = s.split(['x', 'X', '*']).collect();
    if parts.len() == 2 {
        let w = parts[0].parse().unwrap_or(1024);
        let h = parts[1].parse().unwrap_or(1024);
        return (w.max(64), h.max(64));
    }
    (1024, 1024)
}

pub(crate) fn image_to_data_url(bytes: &[u8], mime: &str) -> String {
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
    format!("data:{mime};base64,{b64}")
}
