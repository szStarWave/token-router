use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::types::{
    VideoCreateRequest, VideoErrorObject, VideoJob, now_unix,
};
use crate::gateway::video::{
    image_ref_to_url, join_url, openai_error_message, resolve_model_name, seconds_to_u32,
    size_to_resolution_ratio,
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
    let (resolution, ratio) = size_to_resolution_ratio(req.size.as_deref());
    let duration = seconds_to_u32(req.seconds.as_deref());

    let mut content = vec![json!({
        "type": "text",
        "text": req.prompt,
    })];
    if let Some(reference) = &req.input_reference {
        let url = image_ref_to_url(http, reference).await?;
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": url },
            "role": "first_frame",
        }));
    }

    let body = json!({
        "model": model,
        "content": content,
        "duration": duration,
        "resolution": resolution.to_ascii_lowercase(),
        "ratio": ratio,
    });

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
    let task_status = v
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("queued");

    let mut out = job.clone();
    match task_status.to_ascii_lowercase().as_str() {
        "queued" | "pending" => {
            out.status = "queued".into();
            out.progress = out.progress.max(5);
        }
        "running" | "processing" | "in_progress" => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(40).min(90);
        }
        "succeeded" | "success" | "completed" => {
            out.status = "completed".into();
            out.progress = 100;
            out.result_url = extract_video_url(&v);
            if out.result_url.is_none() {
                out.status = "failed".into();
                out.error = Some(VideoErrorObject {
                    code: Some("missing_video_url".into()),
                    message: Some("seedance succeeded but no video_url".into()),
                });
            }
        }
        "failed" | "cancelled" | "canceled" | "error" => {
            out.status = "failed".into();
            let msg = v
                .pointer("/error/message")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("seedance video task failed");
            out.error = Some(VideoErrorObject {
                code: Some(task_status.to_string()),
                message: Some(msg.to_string()),
            });
        }
        other => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(20);
            let _ = other;
        }
    }
    out.touch();
    Ok(out)
}

fn extract_video_url(v: &Value) -> Option<String> {
    v.pointer("/content/video_url")
        .or_else(|| v.pointer("/content/0/video_url"))
        .or_else(|| v.get("video_url"))
        .and_then(|u| u.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::video::size_to_resolution_ratio;

    #[test]
    fn size_mapping_720p_16_9() {
        let (res, ratio) = size_to_resolution_ratio(Some("1280x720"));
        assert_eq!(res.to_ascii_lowercase(), "720p");
        assert_eq!(ratio, "16:9");
    }

    #[test]
    fn extract_succeeded_fixture() {
        let text = r#"{
            "id": "cgt-xxx",
            "status": "succeeded",
            "content": { "video_url": "https://example.com/out.mp4" }
        }"#;
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            extract_video_url(&v).as_deref(),
            Some("https://example.com/out.mp4")
        );
    }
}
