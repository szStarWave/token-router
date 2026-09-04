//! Video generation providers and local job mapping (OpenAI Videos API shape).

mod comfyui;
mod dashscope;
mod minimax;
mod openai;
mod seedance;
pub mod store;
pub mod tier;
pub mod types;

use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client;

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::edge_load::{EdgeInferenceGuard, EdgeInferenceTracker};
use crate::gateway::error::{AppError, AppResult};

use self::store::VideoJobStore;
use self::types::{ImageRef, VideoCreateRequest, VideoJob, VideoListResponse, VideoObject, new_video_id};

pub use tier::{resolve_video_tier, VideoTier};

const VIDEO_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct VideoClient {
    http: Client,
    edge_load: Arc<EdgeInferenceTracker>,
    store: Arc<VideoJobStore>,
    edge_guards: Arc<Mutex<HashMap<String, EdgeInferenceGuard>>>,
}

impl VideoClient {
    pub fn new(edge_load: Arc<EdgeInferenceTracker>, store: Arc<VideoJobStore>) -> Self {
        let http = Client::builder()
            .timeout(VIDEO_HTTP_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            edge_load,
            store,
            edge_guards: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn store(&self) -> &Arc<VideoJobStore> {
        &self.store
    }

    pub async fn create(
        &self,
        target: &ResolvedVideoUpstream,
        tier: VideoTier,
        req: &VideoCreateRequest,
    ) -> AppResult<VideoObject> {
        let local_id = new_video_id();
        let job = dispatch_create(&self.http, target, req, &local_id, tier.as_str()).await?;
        self.store.save(&job)?;
        if tier == VideoTier::Edge {
            self.hold_edge_guard(&job.id);
        }
        Ok(job.to_object())
    }

    pub async fn refresh(
        &self,
        target: &ResolvedVideoUpstream,
        job: &VideoJob,
    ) -> AppResult<VideoObject> {
        if job.is_terminal() {
            return Ok(job.to_object());
        }
        let updated = dispatch_refresh(&self.http, target, job, &self.store).await?;
        self.store.save(&updated)?;
        if updated.is_terminal() {
            self.release_edge_guard(&updated.id);
        }
        Ok(updated.to_object())
    }

    pub async fn retrieve(
        &self,
        target: Option<&ResolvedVideoUpstream>,
        video_id: &str,
    ) -> AppResult<VideoObject> {
        let job = self.store.require(video_id)?;
        if job.is_terminal() {
            return Ok(job.to_object());
        }
        let Some(target) = target else {
            return Ok(job.to_object());
        };
        self.refresh(target, &job).await
    }

    pub fn list(
        &self,
        limit: usize,
        after: Option<&str>,
        order_desc: bool,
    ) -> AppResult<VideoListResponse> {
        let (jobs, has_more) = self.store.list(limit, after, order_desc)?;
        let data: Vec<VideoObject> = jobs.iter().map(VideoJob::to_object).collect();
        let first_id = data.first().map(|v| v.id.clone());
        let last_id = data.last().map(|v| v.id.clone());
        Ok(VideoListResponse {
            object: "list".into(),
            data,
            first_id,
            last_id,
            has_more,
        })
    }

    pub async fn cancel(
        &self,
        target: Option<&ResolvedVideoUpstream>,
        video_id: &str,
    ) -> AppResult<VideoObject> {
        let mut job = self.store.require(video_id)?;
        if matches!(
            job.status.to_ascii_lowercase().as_str(),
            "cancelled" | "canceled"
        ) {
            return Ok(job.to_object());
        }
        if job.is_terminal() {
            return Err(AppError::BadRequest(format!(
                "video `{video_id}` is already terminal (status={})",
                job.status
            )));
        }
        if let Some(target) = target {
            dispatch_cancel(&self.http, target, &job).await?;
        }
        job.status = "cancelled".into();
        job.error = Some(crate::gateway::video::types::VideoErrorObject {
            code: Some("cancelled".into()),
            message: Some("video cancelled".into()),
        });
        job.touch();
        self.store.save(&job)?;
        self.release_edge_guard(&job.id);
        Ok(job.to_object())
    }

    pub async fn delete(
        &self,
        target: Option<&ResolvedVideoUpstream>,
        video_id: &str,
    ) -> AppResult<()> {
        let job = self.store.require(video_id)?;
        if let Some(target) = target {
            // Best-effort upstream delete/cancel; local cleanup still proceeds on soft failures.
            let _ = dispatch_delete_upstream(&self.http, target, &job).await;
        }
        self.store.delete(video_id)?;
        self.release_edge_guard(video_id);
        Ok(())
    }

    pub async fn download_content(
        &self,
        target: Option<&ResolvedVideoUpstream>,
        video_id: &str,
        variant: Option<&str>,
    ) -> AppResult<(bytes::Bytes, &'static str)> {
        if let Some(v) = variant.map(str::trim).filter(|s| !s.is_empty()) {
            if !v.eq_ignore_ascii_case("video") {
                return Err(AppError::BadRequest(format!(
                    "variant `{v}` not supported in v1 (use video)"
                )));
            }
        }

        let mut job = self.store.require(video_id)?;
        if !job.is_terminal() {
            if let Some(target) = target {
                let _ = self.refresh(target, &job).await?;
                job = self.store.require(video_id)?;
            }
        }
        if job.status != "completed" {
            return Err(AppError::BadRequest(format!(
                "video `{video_id}` is not completed (status={})",
                job.status
            )));
        }

        if let Some(path) = &job.local_path {
            let bytes = fs::read(path)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("read local video: {e}")))?;
            return Ok((bytes::Bytes::from(bytes), "video/mp4"));
        }

        if let Some(url) = &job.result_url {
            let mut builder = self.http.get(url);
            // MiniMax file retrieve endpoints require Bearer auth.
            if job.provider == "minimax" {
                if let Some(key) = target.and_then(|t| t.api_key.as_deref()) {
                    builder = builder.bearer_auth(key);
                }
            }
            let bytes = builder
                .send()
                .await
                .map_err(|e| AppError::Upstream(format!("download video url: {e}")))?
                .error_for_status()
                .map_err(|e| AppError::Upstream(format!("download video url: {e}")))?
                .bytes()
                .await
                .map_err(|e| AppError::Upstream(format!("download video body: {e}")))?;
            // Optional cache
            let path = self.store.local_file_path(video_id);
            if fs::write(&path, &bytes).is_ok() {
                let mut updated = job.clone();
                updated.local_path = Some(path.display().to_string());
                updated.touch();
                let _ = self.store.save(&updated);
            }
            return Ok((bytes, "video/mp4"));
        }

        // OpenAI: proxy content from upstream.
        if job.provider == "openai" {
            let target = target.ok_or_else(|| {
                AppError::Unavailable("openai video upstream missing for content".into())
            })?;
            let bytes = openai::download_content(&self.http, target, &job).await?;
            return Ok((bytes, "video/mp4"));
        }

        Err(AppError::Upstream(format!(
            "video `{video_id}` has no downloadable content"
        )))
    }

    fn hold_edge_guard(&self, video_id: &str) {
        let guard = self.edge_load.begin();
        if let Ok(mut map) = self.edge_guards.lock() {
            map.insert(video_id.to_string(), guard);
        }
    }

    fn release_edge_guard(&self, video_id: &str) {
        if let Ok(mut map) = self.edge_guards.lock() {
            map.remove(video_id);
        }
    }
}

async fn dispatch_create(
    http: &Client,
    target: &ResolvedVideoUpstream,
    req: &VideoCreateRequest,
    local_id: &str,
    tier: &str,
) -> AppResult<VideoJob> {
    match target.provider.as_str() {
        "openai" => openai::create(http, target, req, local_id, tier).await,
        "dashscope" => dashscope::create(http, target, req, local_id, tier).await,
        "seedance" => seedance::create(http, target, req, local_id, tier).await,
        "minimax" => minimax::create(http, target, req, local_id, tier).await,
        "comfyui" => comfyui::create(http, target, req, local_id, tier).await,
        other => Err(AppError::BadRequest(format!(
            "unsupported video provider `{other}`"
        ))),
    }
}

async fn dispatch_refresh(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
    store: &VideoJobStore,
) -> AppResult<VideoJob> {
    match job.provider.as_str() {
        "openai" => openai::refresh(http, target, job).await,
        "dashscope" => dashscope::refresh(http, target, job).await,
        "seedance" => seedance::refresh(http, target, job).await,
        "minimax" => minimax::refresh(http, target, job).await,
        "comfyui" => comfyui::refresh(http, target, job, store).await,
        other => Err(AppError::BadRequest(format!(
            "unsupported video provider `{other}`"
        ))),
    }
}

async fn dispatch_cancel(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
) -> AppResult<()> {
    match job.provider.as_str() {
        "minimax" => minimax::cancel(http, target, job).await,
        // Other providers: local cancel only (no upstream cancel API wired).
        "openai" | "dashscope" | "seedance" | "comfyui" => Ok(()),
        other => Err(AppError::BadRequest(format!(
            "unsupported video provider `{other}`"
        ))),
    }
}

async fn dispatch_delete_upstream(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
) -> AppResult<()> {
    match job.provider.as_str() {
        "minimax" => minimax::delete_upstream_task(http, target, job).await,
        _ => Ok(()),
    }
}

pub(crate) fn resolve_model_name(
    target: &ResolvedVideoUpstream,
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

pub(crate) fn parse_wxh(size: Option<&str>) -> (u32, u32) {
    let s = size.unwrap_or("1280x720");
    let parts: Vec<&str> = s.split(['x', 'X', '*']).collect();
    if parts.len() == 2 {
        let w = parts[0].parse().unwrap_or(1280);
        let h = parts[1].parse().unwrap_or(720);
        return (w.max(64), h.max(64));
    }
    (1280, 720)
}

pub(crate) fn seconds_to_u32(seconds: Option<&str>) -> u32 {
    seconds
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(5)
}

/// Map OpenAI-style `WxH` to provider resolution + aspect ratio.
pub fn size_to_resolution_ratio(size: Option<&str>) -> (String, String) {
    let (w, h) = parse_wxh(size);
    let pixels = w.saturating_mul(h);
    let resolution = if pixels >= 1_800_000 {
        "1080P"
    } else if pixels >= 800_000 {
        "720P"
    } else {
        "480P"
    };
    (resolution.to_string(), snap_aspect_ratio(w, h))
}

/// Snap WxH to a common aspect-ratio string.
pub(crate) fn snap_aspect_ratio(w: u32, h: u32) -> String {
    let a = w as f32 / h.max(1) as f32;
    if (a - 21.0 / 9.0).abs() < 0.08 {
        "21:9".into()
    } else if (a - 16.0 / 9.0).abs() < 0.08 {
        "16:9".into()
    } else if (a - 9.0 / 16.0).abs() < 0.08 {
        "9:16".into()
    } else if (a - 4.0 / 3.0).abs() < 0.08 {
        "4:3".into()
    } else if (a - 3.0 / 4.0).abs() < 0.08 {
        "3:4".into()
    } else if (a - 1.0).abs() < 0.08 {
        "1:1".into()
    } else {
        "16:9".into()
    }
}

/// Snap OpenAI `WxH` to DashScope wan2.6-and-earlier `parameters.size` (`W*H`).
pub(crate) fn openai_size_to_dashscope_size(size: Option<&str>) -> String {
    let (w, h) = parse_wxh(size);
    let ratio = snap_aspect_ratio(w, h);
    let pixels = w.saturating_mul(h);
    let tier = if pixels >= 1_800_000 {
        "1080P"
    } else if pixels >= 800_000 {
        "720P"
    } else {
        "480P"
    };
    match (tier, ratio.as_str()) {
        ("1080P", "16:9") => "1920*1080".into(),
        ("1080P", "9:16") => "1080*1920".into(),
        ("1080P", "1:1") => "1440*1440".into(),
        ("1080P", "4:3") => "1632*1248".into(),
        ("1080P", "3:4") => "1248*1632".into(),
        ("720P", "16:9") => "1280*720".into(),
        ("720P", "9:16") => "720*1280".into(),
        ("720P", "1:1") => "960*960".into(),
        ("720P", "4:3") => "1088*832".into(),
        ("720P", "3:4") => "832*1088".into(),
        ("480P", "16:9") => "832*480".into(),
        ("480P", "9:16") => "480*832".into(),
        ("480P", "1:1") => "624*624".into(),
        _ => "1280*720".into(),
    }
}

/// Snap ratio to DashScope wan2.7+ allowed set (no 21:9).
pub(crate) fn snap_dashscope_ratio(ratio: &str) -> String {
    match ratio {
        "16:9" | "9:16" | "1:1" | "4:3" | "3:4" => ratio.to_string(),
        "21:9" => "16:9".into(),
        _ => "16:9".into(),
    }
}

pub(crate) async fn image_ref_to_url(http: &Client, reference: &ImageRef) -> AppResult<String> {
    match reference {
        ImageRef::Url(url) => Ok(url.clone()),
        ImageRef::Bytes { bytes, mime, .. } => {
            let _ = http; // bytes already in memory
            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
            Ok(format!("data:{mime};base64,{b64}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::video::types::is_terminal_status;

    #[test]
    fn size_1280x720_maps_720p_16_9() {
        let (res, ratio) = size_to_resolution_ratio(Some("1280x720"));
        assert_eq!(res, "720P");
        assert_eq!(ratio, "16:9");
    }

    #[test]
    fn size_1080p_maps() {
        let (res, ratio) = size_to_resolution_ratio(Some("1920x1080"));
        assert_eq!(res, "1080P");
        assert_eq!(ratio, "16:9");
    }

    #[test]
    fn dashscope_size_snaps() {
        assert_eq!(
            openai_size_to_dashscope_size(Some("1280x720")),
            "1280*720"
        );
        assert_eq!(
            openai_size_to_dashscope_size(Some("1920x1080")),
            "1920*1080"
        );
        assert_eq!(
            openai_size_to_dashscope_size(Some("720x1280")),
            "720*1280"
        );
    }

    #[test]
    fn terminal_statuses() {
        assert!(is_terminal_status("completed"));
        assert!(is_terminal_status("failed"));
        assert!(is_terminal_status("cancelled"));
        assert!(is_terminal_status("canceled"));
        assert!(!is_terminal_status("queued"));
    }
}
