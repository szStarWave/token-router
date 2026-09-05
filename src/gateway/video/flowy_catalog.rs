//! Flowy claw video catalog outbound (OpenRouter-style by model name).
//!
//! Herdsman configures `provider=openai` + `base_url=…/claw/v1`. That must NOT
//! call OpenAI `{base}/videos` (empty body → EOF). Flowy video lives on the
//! business root `/claw` (strip `/v1`):
//!
//!   POST   {claw}/video/generations/tasks
//!   GET    {claw}/video/generations/tasks/{id}
//!   DELETE {claw}/video/generations/tasks/{id}
//!
//! Model id (e.g. `MiniMax-H3` / `flowy/MiniMax-H3`) selects the upstream channel.

use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::minimax::{
    normalize_ratio, normalize_resolution_for_model, size_to_ratio,
};
use crate::gateway::video::types::{
    ImageRef, VideoCreateRequest, VideoErrorObject, VideoJob, now_unix,
};
use crate::gateway::video::{
    image_ref_to_url, join_url, openai_error_message, resolve_model_name, seconds_to_u32,
};

const CREATE_PATH: &str = "video/generations/tasks";
const USER_AGENT: &str = "token-router-videogen/1.0";
const MAX_REFERENCE_IMAGES: usize = 9;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// True when the configured video base is Flowy claw (catalog), not OpenAI / MiniMax official.
pub fn is_flowy_catalog_base(base_url: &str) -> bool {
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    lower.contains("flowyaipc")
        || lower.contains("/claw/")
        || lower.ends_with("/claw")
        || lower.contains("claw/v1")
}

/// `https://server.flowyaipc.cn/claw/v1` → `https://server.flowyaipc.cn/claw`
pub fn flowy_business_base(configured_base: &str) -> String {
    let base = configured_base.trim().trim_end_matches('/');
    if let Some(stripped) = base
        .strip_suffix("/v1")
        .or_else(|| base.strip_suffix("/V1"))
    {
        return stripped.trim_end_matches('/').to_string();
    }
    base.to_string()
}

/// Ensure Flowy catalog model ids (`flowy/…` / list ids). Bare `MiniMax-H3` → `flowy/MiniMax-H3`.
pub fn normalize_flowy_model(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() {
        return "flowy/MiniMax-H3".into();
    }
    let lower = m.to_ascii_lowercase();
    if lower.starts_with("flowy/") || lower.starts_with("aipc-") {
        return m.to_string();
    }
    if m.contains('/') {
        return m.to_string();
    }
    format!("flowy/{m}")
}

pub async fn create(
    http: &Client,
    target: &ResolvedVideoUpstream,
    req: &VideoCreateRequest,
    local_id: &str,
    tier: &str,
) -> AppResult<VideoJob> {
    let configured = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video flowy catalog base_url missing".into()))?;
    let business = flowy_business_base(configured);
    let api_key = target.api_key.as_deref();
    let raw_model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "MiniMax-H3".to_string());
    let model = normalize_flowy_model(&raw_model);
    let duration = seconds_to_u32(req.seconds.as_deref()).clamp(4, 15);

    let has_first = req.input_reference.is_some();
    let has_last = req.last_frame.is_some();
    let refs: Vec<&ImageRef> = req.reference_images.iter().take(MAX_REFERENCE_IMAGES).collect();
    let has_reference = !refs.is_empty();
    if (has_first || has_last) && has_reference {
        return Err(AppError::BadRequest(
            "first/last frame cannot be mixed with reference media".into(),
        ));
    }
    let has_media = has_first || has_last || has_reference;
    let resolution = normalize_resolution_for_model(
        &raw_model,
        req.resolution.as_deref(),
        req.size.as_deref(),
    );
    let ratio = normalize_ratio(size_to_ratio(req.size.as_deref()), has_media);

    let mut content = vec![json!({
        "type": "text",
        "text": req.prompt,
    })];

    if let Some(reference) = &req.input_reference {
        let url = image_ref_to_url(http, reference).await?;
        content.push(json!({
            "type": "image_url",
            "role": "first_frame",
            "image_url": { "url": url },
        }));
    }
    if let Some(last) = &req.last_frame {
        let url = image_ref_to_url(http, last).await?;
        content.push(json!({
            "type": "image_url",
            "role": "last_frame",
            "image_url": { "url": url },
        }));
    }
    for reference in refs {
        let url = image_ref_to_url(http, reference).await?;
        content.push(json!({
            "type": "image_url",
            "role": "reference_image",
            "image_url": { "url": url },
        }));
    }

    // MiniMax-H3 (and Flowy catalog peers that accept MiniMax V2 schema).
    let mut body = json!({
        "model": model,
        "content": content,
        "resolution": resolution,
        "duration": duration,
        "ratio": ratio,
        "app": "token-router",
    });
    if req.watermark == Some(true) {
        body["aigc_watermark"] = json!(true);
    }

    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("flowy catalog serialize: {e}")))?;
    if body_bytes.len() > MAX_BODY_BYTES {
        return Err(AppError::BadRequest("request body exceeds 64MB".into()));
    }

    let url = join_url(&business, CREATE_PATH);
    let text = send_json(http, "POST", &url, api_key, Some(body_bytes)).await?;
    let v = parse_business_json(&text, "create")?;
    let task_id = extract_local_task_id(&v).ok_or_else(|| {
        AppError::Upstream(format!("flowy catalog missing task id: {text}"))
    })?;

    let now = now_unix();
    Ok(VideoJob {
        // Keep provider=openai so config/dispatch stay consistent; refresh
        // re-detects Flowy catalog from base_url.
        id: local_id.to_string(),
        provider: "openai".into(),
        tier: tier.into(),
        upstream_task_id: Some(task_id),
        status: "queued".into(),
        progress: 0,
        model,
        seconds: req.seconds.clone().or_else(|| Some(duration.to_string())),
        size: req.size.clone(),
        prompt: Some(req.prompt.clone()),
        error: None,
        result_url: None,
        local_path: None,
        created_at: now,
        updated_at: now,
    })
}

pub async fn refresh(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
) -> AppResult<VideoJob> {
    let configured = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video flowy catalog base_url missing".into()))?;
    let business = flowy_business_base(configured);
    let task_id = job
        .upstream_task_id
        .as_deref()
        .ok_or_else(|| AppError::Upstream("flowy catalog missing task id".into()))?;
    let url = join_url(&business, &format!("{CREATE_PATH}/{task_id}"));
    let text = send_json(http, "GET", &url, target.api_key.as_deref(), None).await?;
    let v = parse_business_json(&text, "query")?;
    Ok(apply_flowy_query_to_job(job, &v))
}

pub async fn cancel(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
) -> AppResult<()> {
    let status = job.status.to_ascii_lowercase();
    if matches!(status.as_str(), "cancelled" | "canceled") {
        return Ok(());
    }
    if matches!(status.as_str(), "completed" | "failed") {
        return Err(AppError::BadRequest(format!(
            "video `{}` is already terminal (status={})",
            job.id, job.status
        )));
    }
    // Flowy rejects DELETE while still generating (HTTP 409).
    if matches!(
        status.as_str(),
        "in_progress" | "running" | "processing" | "generating"
    ) {
        return Err(AppError::BadRequest(
            "flowy catalog cannot cancel a running video task".into(),
        ));
    }
    delete_upstream_task(http, target, job).await
}

pub async fn delete_upstream_task(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
) -> AppResult<()> {
    let Some(task_id) = job.upstream_task_id.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let configured = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video flowy catalog base_url missing".into()))?;
    let business = flowy_business_base(configured);
    let url = join_url(&business, &format!("{CREATE_PATH}/{task_id}"));
    let mut builder = http
        .delete(&url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key).header("token", key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("flowy catalog delete: {e}")))?;
    let status = resp.status();
    if status.is_success() || status.as_u16() == 404 {
        return Ok(());
    }
    let text = resp.text().await.unwrap_or_default();
    Err(AppError::Upstream(openai_error_message(status, &text)))
}

async fn send_json(
    http: &Client,
    method: &str,
    url: &str,
    api_key: Option<&str>,
    body: Option<Vec<u8>>,
) -> AppResult<String> {
    let mut builder = match method {
        "GET" => http.get(url),
        "POST" => http.post(url),
        other => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "unsupported method {other}"
            )));
        }
    };
    builder = builder
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT);
    if let Some(bytes) = body {
        builder = builder
            .header("Content-Type", "application/json")
            .body(bytes);
    }
    if let Some(key) = api_key {
        builder = builder.bearer_auth(key).header("token", key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("flowy catalog {method}: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("flowy catalog {method} body: {e}")))?;
    if text.trim().is_empty() {
        return Err(AppError::Upstream(format!(
            "flowy catalog {method} empty body (HTTP {status}); refused OpenAI /videos on claw/v1"
        )));
    }
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }
    Ok(text)
}

fn parse_business_json(text: &str, op: &str) -> AppResult<Value> {
    let v: Value = serde_json::from_str(text).map_err(|e| {
        AppError::Upstream(format!(
            "flowy catalog {op} json: {e} (body_len={})",
            text.len()
        ))
    })?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(200);
    if code != 200 {
        let msg = v
            .get("msg")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown error");
        return Err(AppError::Upstream(format!(
            "flowy catalog {op} failed (code={code}): {msg}"
        )));
    }
    Ok(v)
}

fn extract_local_task_id(v: &Value) -> Option<String> {
    let data = v.get("data").unwrap_or(v);
    if let Some(id) = data.get("id") {
        if let Some(n) = id.as_i64() {
            return Some(n.to_string());
        }
        if let Some(n) = id.as_u64() {
            return Some(n.to_string());
        }
        if let Some(s) = id.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
    }
    None
}

fn apply_flowy_query_to_job(job: &VideoJob, v: &Value) -> VideoJob {
    let data = v.get("data").unwrap_or(v);
    let mut out = job.clone();
    let status_code = data
        .get("status")
        .and_then(|s| s.as_i64().or_else(|| s.as_u64().map(|u| u as i64)));
    match status_code {
        Some(1) => {
            out.status = "queued".into();
            out.progress = out.progress.max(5);
            out.error = None;
        }
        Some(2) => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(40).min(90);
            out.error = None;
        }
        Some(3) => {
            out.status = "cancelled".into();
            out.error = Some(VideoErrorObject {
                code: Some("cancelled".into()),
                message: Some("video cancelled".into()),
            });
        }
        Some(4) => {
            out.status = "completed".into();
            out.progress = 100;
            out.result_url = extract_video_url(data);
            out.error = None;
            if out.result_url.is_none() {
                out.status = "failed".into();
                out.error = Some(VideoErrorObject {
                    code: Some("missing_video_url".into()),
                    message: Some("flowy catalog succeeded without video_url".into()),
                });
            }
        }
        Some(5) | Some(6) => {
            out.status = "failed".into();
            out.error = Some(VideoErrorObject {
                code: Some(if status_code == Some(6) {
                    "expired".into()
                } else {
                    "failed".into()
                }),
                message: Some(
                    data
                        .pointer("/result/base_resp/status_msg")
                        .and_then(|m| m.as_str())
                        .or_else(|| data.get("msg").and_then(|m| m.as_str()))
                        .unwrap_or("video generation failed")
                        .to_string(),
                ),
            });
        }
        _ => {
            // Fall back to nested MiniMax-style result.status if present.
            if let Some(s) = data
                .pointer("/result/status")
                .and_then(|s| s.as_str())
            {
                match s.to_ascii_lowercase().as_str() {
                    "queued" | "pending" => {
                        out.status = "queued".into();
                        out.progress = out.progress.max(5);
                    }
                    "running" | "processing" => {
                        out.status = "in_progress".into();
                        out.progress = out.progress.max(40).min(90);
                    }
                    "success" | "succeeded" | "completed" => {
                        out.status = "completed".into();
                        out.progress = 100;
                        out.result_url = extract_video_url(data);
                    }
                    "failed" | "error" => {
                        out.status = "failed".into();
                    }
                    _ => {}
                }
            }
        }
    }
    out.touch();
    out
}

fn extract_video_url(data: &Value) -> Option<String> {
    data.pointer("/result/content/video_url")
        .and_then(|u| u.as_str())
        .or_else(|| data.pointer("/result/content/url").and_then(|u| u.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_flowy_claw_bases() {
        assert!(is_flowy_catalog_base(
            "https://server.flowyaipc.cn/claw/v1"
        ));
        assert!(is_flowy_catalog_base("https://server.flowyaipc.cn/claw"));
        assert!(is_flowy_catalog_base("https://x.example/claw/v1/"));
        assert!(!is_flowy_catalog_base("https://api.minimax.io"));
        assert!(!is_flowy_catalog_base("https://api.openai.com/v1"));
    }

    #[test]
    fn strips_v1_to_business_root() {
        assert_eq!(
            flowy_business_base("https://server.flowyaipc.cn/claw/v1"),
            "https://server.flowyaipc.cn/claw"
        );
        assert_eq!(
            flowy_business_base("https://server.flowyaipc.cn/claw/v1/"),
            "https://server.flowyaipc.cn/claw"
        );
        assert_eq!(
            flowy_business_base("https://server.flowyaipc.cn/claw"),
            "https://server.flowyaipc.cn/claw"
        );
    }

    #[test]
    fn normalizes_catalog_model_names() {
        assert_eq!(normalize_flowy_model("MiniMax-H3"), "flowy/MiniMax-H3");
        assert_eq!(
            normalize_flowy_model("flowy/MiniMax-H3"),
            "flowy/MiniMax-H3"
        );
        assert_eq!(
            normalize_flowy_model("AIPC-abc123"),
            "AIPC-abc123"
        );
    }

    #[test]
    fn create_url_never_uses_openai_videos() {
        let business = flowy_business_base("https://server.flowyaipc.cn/claw/v1");
        let url = join_url(&business, CREATE_PATH);
        assert_eq!(
            url,
            "https://server.flowyaipc.cn/claw/video/generations/tasks"
        );
        assert!(!url.contains("/claw/v1/videos"));
        assert!(!url.contains("/v2/video_generation"));
    }

    #[test]
    fn maps_status_codes() {
        let job = VideoJob {
            id: "local".into(),
            provider: "openai".into(),
            tier: "cloud".into(),
            upstream_task_id: Some("99".into()),
            status: "queued".into(),
            progress: 0,
            model: "flowy/MiniMax-H3".into(),
            seconds: Some("5".into()),
            size: None,
            prompt: None,
            error: None,
            result_url: None,
            local_path: None,
            created_at: 1,
            updated_at: 1,
        };
        let queued = json!({"code":200,"data":{"id":99,"status":1}});
        let out = apply_flowy_query_to_job(&job, &queued);
        assert_eq!(out.status, "queued");

        let done = json!({
            "code": 200,
            "data": {
                "id": 99,
                "status": 4,
                "result": { "content": { "video_url": "https://cdn.example/out.mp4" } }
            }
        });
        let out = apply_flowy_query_to_job(&job, &done);
        assert_eq!(out.status, "completed");
        assert_eq!(out.result_url.as_deref(), Some("https://cdn.example/out.mp4"));
    }
}
