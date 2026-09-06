//! Volcengine Ark / Seedance video generation adapter.
//! Maps OpenAI Videos create/poll into `contents/generations/tasks`.

use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::types::{
    VideoCreateRequest, VideoErrorObject, VideoJob, VideoObject, now_unix,
};
use crate::gateway::video::{
    image_ref_to_url, join_url, openai_error_message, resolve_model_name, seconds_to_u32,
    size_to_resolution_ratio, snap_aspect_ratio, parse_wxh,
};

pub async fn create(
    http: &Client,
    target: &ResolvedVideoUpstream,
    req: &VideoCreateRequest,
    local_id: &str,
    tier: &str,
) -> AppResult<VideoJob> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video seedance base_url missing".into()))?;
    let model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "doubao-seedance-1-0-pro-250528".to_string());

    let first_url = if let Some(reference) = &req.input_reference {
        Some(image_ref_to_url(http, reference).await?)
    } else {
        None
    };
    let last_url = if let Some(last) = &req.last_frame {
        Some(image_ref_to_url(http, last).await?)
    } else {
        None
    };

    let body = build_create_body(
        &model,
        &req.prompt,
        req.size.as_deref(),
        req.seconds.as_deref(),
        first_url.as_deref(),
        last_url.as_deref(),
        req.watermark,
        req.resolution.as_deref(),
    );
    let duration = body["duration"].as_u64().unwrap_or(5) as u32;

    let url = join_url(base, "contents/generations/tasks");
    let mut builder = http.post(&url).json(&body);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("seedance create: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("seedance create body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("seedance create json: {e}")))?;
    let task_id = v
        .get("id")
        .or_else(|| v.get("task_id"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| AppError::Upstream(format!("seedance missing task id: {text}")))?
        .to_string();

    let now = now_unix();
    Ok(VideoJob {
        id: local_id.to_string(),
        provider: "seedance".into(),
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
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video seedance base_url missing".into()))?;
    let task_id = job
        .upstream_task_id
        .as_deref()
        .ok_or_else(|| AppError::Upstream("seedance missing task id".into()))?;
    let url = join_url(base, &format!("contents/generations/tasks/{task_id}"));
    let mut builder = http.get(&url);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("seedance task: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("seedance task body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("seedance task json: {e}")))?;
    Ok(apply_query_to_job(job, &v))
}

pub(crate) fn build_create_body(
    model: &str,
    prompt: &str,
    size: Option<&str>,
    seconds: Option<&str>,
    first_url: Option<&str>,
    last_url: Option<&str>,
    watermark: Option<bool>,
    resolution: Option<&str>,
) -> Value {
    let (_, ratio_from_size) = size_to_resolution_ratio(size);
    let has_media = first_url.is_some() || last_url.is_some();
    let ratio = if has_media {
        "adaptive".to_string()
    } else {
        // Seedance supports 21:9; keep snap from WxH.
        let (w, h) = parse_wxh(size);
        let snapped = snap_aspect_ratio(w, h);
        match snapped.as_str() {
            "21:9" | "16:9" | "4:3" | "1:1" | "3:4" | "9:16" => snapped,
            _ => ratio_from_size,
        }
    };
    let duration = clamp_duration(model, seconds_to_u32(seconds));

    let mut content = vec![json!({
        "type": "text",
        "text": prompt,
    })];
    if let Some(url) = first_url {
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": url },
            "role": "first_frame",
        }));
    }
    if let Some(url) = last_url {
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": url },
            "role": "last_frame",
        }));
    }

    json!({
        "model": model,
        "content": content,
        "duration": duration,
        "resolution": normalize_seedance_resolution(resolution, size),
        "ratio": ratio,
        "watermark": watermark.unwrap_or(false),
    })
}

/// True for Seedance / Doubao Seedance catalog or direct Ark models.
pub(crate) fn is_seedance_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("seedance") || lower.contains("doubao-seedance")
}

fn model_is_seedance_2(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("seedance-2")
        || lower.contains("seedance_2")
        || lower.contains("seedance-2.")
        || lower.contains("seedance2")
}

fn clamp_duration(model: &str, secs: u32) -> u32 {
    if model_is_seedance_2(model) {
        secs.clamp(4, 15)
    } else {
        // Seedance 1.0 family: typically 2–12s.
        secs.clamp(2, 12)
    }
}

/// Prefer explicit inbound `resolution`; else derive from OpenAI `size`.
/// Wire values are lowercase Ark labels (`480p` / `720p` / `1080p` / `4k`).
pub(crate) fn normalize_seedance_resolution(
    resolution: Option<&str>,
    size: Option<&str>,
) -> String {
    let raw = if let Some(r) = resolution.map(str::trim).filter(|s| !s.is_empty()) {
        r.to_string()
    } else {
        size_to_resolution_ratio(size).0
    };
    snap_seedance_resolution(&raw)
}

fn snap_seedance_resolution(resolution: &str) -> String {
    let upper = resolution.trim().to_ascii_uppercase().replace('_', "");
    match upper.as_str() {
        "1080P" | "1080" | "2K" => "1080p".into(),
        "480P" | "480" => "480p".into(),
        "4K" | "2160P" | "2160" => "4k".into(),
        // MiniMax-style 768P and OpenAI 720p both map to Seedance 720p.
        "720P" | "720" | "768P" | "768" | "MEDIUM" | "AUTO" | "" => "720p".into(),
        _ => "720p".into(),
    }
}

pub(crate) fn apply_query_to_job(job: &VideoJob, v: &Value) -> VideoJob {
    let task_status = v
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("queued");

    let mut out = job.clone();
    match task_status.to_ascii_lowercase().as_str() {
        "queued" | "pending" => {
            out.status = "queued".into();
            out.progress = out.progress.max(5);
            out.error = None;
        }
        "running" | "processing" | "in_progress" => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(40).min(90);
            out.error = None;
        }
        "succeeded" | "success" | "completed" => {
            out.status = "completed".into();
            out.progress = 100;
            out.result_url = extract_video_url(v);
            out.error = None;
            if out.seconds.is_none() {
                if let Some(d) = v.get("duration").and_then(|d| d.as_u64()) {
                    out.seconds = Some(d.to_string());
                }
            }
            if out.result_url.is_none() {
                out.status = "failed".into();
                out.error = Some(VideoErrorObject {
                    code: Some("missing_video_url".into()),
                    message: Some("seedance succeeded but no video_url".into()),
                });
            }
        }
        "cancelled" | "canceled" => {
            out.status = "cancelled".into();
            out.error = Some(VideoErrorObject {
                code: Some("cancelled".into()),
                message: Some(extract_error_message(v).unwrap_or_else(|| {
                    "seedance video task cancelled".into()
                })),
            });
        }
        "failed" | "error" => {
            out.status = "failed".into();
            out.error = Some(VideoErrorObject {
                code: Some(task_status.to_string()),
                message: Some(extract_error_message(v).unwrap_or_else(|| {
                    "seedance video task failed".into()
                })),
            });
        }
        _ => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(20);
        }
    }
    out.touch();
    out
}

fn extract_video_url(v: &Value) -> Option<String> {
    v.pointer("/content/video_url")
        .or_else(|| v.pointer("/content/0/video_url"))
        .or_else(|| v.get("video_url"))
        .and_then(|u| u.as_str())
        .map(str::to_string)
}

fn extract_error_message(v: &Value) -> Option<String> {
    v.pointer("/error/message")
        .or_else(|| v.get("message"))
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_mapping_720p_16_9() {
        let (res, ratio) = size_to_resolution_ratio(Some("1280x720"));
        assert_eq!(res.to_ascii_lowercase(), "720p");
        assert_eq!(ratio, "16:9");
    }

    #[test]
    fn build_body_watermark_and_last_frame() {
        let body = build_create_body(
            "doubao-seedance-2-0-260128",
            "a cat",
            Some("1280x720"),
            Some("6"),
            Some("https://a/first.png"),
            Some("https://a/last.png"),
            Some(true),
            Some("720p"),
        );
        assert_eq!(body["watermark"], json!(true));
        assert_eq!(body["duration"], json!(6));
        assert_eq!(body["resolution"], json!("720p"));
        assert_eq!(body["ratio"], json!("adaptive"));
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[1]["role"], json!("first_frame"));
        assert_eq!(content[2]["role"], json!("last_frame"));
    }

    #[test]
    fn resolution_prefers_inbound_and_maps_minimax_labels() {
        assert_eq!(
            normalize_seedance_resolution(Some("720p"), Some("1920x1080")),
            "720p"
        );
        assert_eq!(
            normalize_seedance_resolution(Some("768P"), None),
            "720p"
        );
        assert_eq!(
            normalize_seedance_resolution(Some("2K"), None),
            "1080p"
        );
        assert_eq!(
            normalize_seedance_resolution(None, Some("1280x720")),
            "720p"
        );
        assert!(is_seedance_model("AIPC-Doubao-Seedance-2.0"));
        assert!(!is_seedance_model("AIPC-MiniMax-H3"));
    }

    #[test]
    fn duration_clamp_by_family() {
        let v1 = build_create_body(
            "doubao-seedance-1-0-pro-250528",
            "x",
            None,
            Some("99"),
            None,
            None,
            None,
            None,
        );
        assert_eq!(v1["duration"], json!(12));

        let v2 = build_create_body(
            "doubao-seedance-2-0-260128",
            "x",
            None,
            Some("1"),
            None,
            None,
            None,
            None,
        );
        assert_eq!(v2["duration"], json!(4));
    }

    #[test]
    fn apply_query_cancelled_not_failed() {
        let job = VideoJob {
            id: "video_x".into(),
            provider: "seedance".into(),
            tier: "cloud".into(),
            upstream_task_id: Some("cgt-1".into()),
            status: "in_progress".into(),
            progress: 40,
            model: "doubao-seedance-1-0-pro-250528".into(),
            seconds: Some("5".into()),
            size: None,
            prompt: None,
            error: None,
            result_url: None,
            local_path: None,
            created_at: 1,
            updated_at: 1,
        };
        let v: Value = serde_json::from_str(r#"{"id":"cgt-1","status":"cancelled"}"#).unwrap();
        let out = apply_query_to_job(&job, &v);
        assert_eq!(out.status, "cancelled");
        let obj = VideoObject::from_job(&out);
        assert_eq!(obj.status, "cancelled");
    }

    #[test]
    fn apply_query_succeeded_openai_shape() {
        let job = VideoJob {
            id: "video_x".into(),
            provider: "seedance".into(),
            tier: "cloud".into(),
            upstream_task_id: Some("cgt-xxx".into()),
            status: "queued".into(),
            progress: 0,
            model: "doubao-seedance-1-0-pro-250528".into(),
            seconds: Some("5".into()),
            size: Some("1280x720".into()),
            prompt: Some("hi".into()),
            error: None,
            result_url: None,
            local_path: None,
            created_at: 1,
            updated_at: 1,
        };
        let v: Value = serde_json::from_str(
            r#"{
            "id": "cgt-xxx",
            "status": "succeeded",
            "content": { "video_url": "https://example.com/out.mp4" }
        }"#,
        )
        .unwrap();
        let out = apply_query_to_job(&job, &v);
        assert_eq!(out.status, "completed");
        assert_eq!(out.result_url.as_deref(), Some("https://example.com/out.mp4"));
        let ser = serde_json::to_value(VideoObject::from_job(&out)).unwrap();
        assert_eq!(ser["object"], "video");
        assert!(ser.get("result_url").is_none());
        assert_eq!(ser["id"], "video_x");
    }
}
