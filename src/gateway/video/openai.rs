use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::types::{
    ImageRef, VideoCreateRequest, VideoErrorObject, VideoJob, now_unix,
};
use crate::gateway::video::{join_url, openai_error_message, resolve_model_name};

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
        .ok_or_else(|| AppError::Unavailable("video openai base_url missing".into()))?;
    let url = join_url(base, "videos");
    let model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "sora-2".to_string());

    let resp_text = if let Some(ImageRef::Bytes {
        bytes,
        mime,
        filename,
    }) = &req.input_reference
    {
        let mut form = reqwest::multipart::Form::new()
            .text("prompt", req.prompt.clone())
            .text("model", model.clone());
        if let Some(seconds) = &req.seconds {
            form = form.text("seconds", seconds.clone());
        }
        if let Some(size) = &req.size {
            form = form.text("size", size.clone());
        }
        let part = reqwest::multipart::Part::bytes(bytes.clone())
            .file_name(filename.clone())
            .mime_str(mime)
            .map_err(|e| AppError::BadRequest(format!("invalid input_reference mime: {e}")))?;
        form = form.part("input_reference", part);
        let mut builder = http.post(&url).multipart(form);
        if let Some(key) = &target.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("openai videos create: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Upstream(format!("openai videos create body: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Upstream(openai_error_message(status, &text)));
        }
        text
    } else {
        let mut body = json!({
            "model": model,
            "prompt": req.prompt,
        });
        if let Some(seconds) = &req.seconds {
            body["seconds"] = json!(seconds);
        }
        if let Some(size) = &req.size {
            body["size"] = json!(size);
        }
        if let Some(ImageRef::Url(image_url)) = &req.input_reference {
            body["input_reference"] = json!({ "image_url": image_url });
        }
        let mut builder = http.post(&url).json(&body);
        if let Some(key) = &target.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("openai videos create: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Upstream(format!("openai videos create body: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Upstream(openai_error_message(status, &text)));
        }
        text
    };

    let upstream = parse_video_json(&resp_text)?;
    let id = upstream
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or(local_id)
        .to_string();
    let now = now_unix();
    Ok(VideoJob {
        id,
        provider: "openai".into(),
        tier: tier.into(),
        upstream_task_id: upstream
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        status: upstream
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("queued")
            .to_string(),
        progress: upstream
            .get("progress")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        model: upstream
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(&model)
            .to_string(),
        seconds: upstream
            .get("seconds")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| req.seconds.clone()),
        size: upstream
            .get("size")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| req.size.clone()),
        prompt: Some(req.prompt.clone()),
        error: parse_error(upstream.get("error")),
        result_url: None,
        local_path: None,
        created_at: upstream
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(now),
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
        .ok_or_else(|| AppError::Unavailable("video openai base_url missing".into()))?;
    let upstream_id = job
        .upstream_task_id
        .as_deref()
        .unwrap_or(job.id.as_str());
    let url = join_url(base, &format!("videos/{upstream_id}"));
    let mut builder = http.get(&url);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("openai videos retrieve: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("openai videos retrieve body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }
    let upstream = parse_video_json(&text)?;
    let mut out = job.clone();
    out.status = upstream
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or(&job.status)
        .to_string();
    out.progress = upstream
        .get("progress")
        .and_then(|v| v.as_u64())
        .unwrap_or(job.progress as u64) as u32;
    out.error = parse_error(upstream.get("error"));
    out.touch();
    Ok(out)
}

pub async fn download_content(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
) -> AppResult<bytes::Bytes> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video openai base_url missing".into()))?;
    let upstream_id = job
        .upstream_task_id
        .as_deref()
        .unwrap_or(job.id.as_str());
    let url = join_url(base, &format!("videos/{upstream_id}/content"));
    let mut builder = http.get(&url);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("openai videos content: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }
    resp.bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("openai videos content body: {e}")))
}

fn parse_video_json(text: &str) -> AppResult<Value> {
    if text.trim().is_empty() {
        return Err(AppError::Upstream(
            "openai videos json: EOF (empty body). If base_url is Flowy …/claw/v1, use catalog outbound (/claw/video/generations/tasks), not POST {base}/videos".into(),
        ));
    }
    serde_json::from_str(text)
        .map_err(|e| AppError::Upstream(format!("openai videos json: {e}")))
}

fn parse_error(v: Option<&Value>) -> Option<VideoErrorObject> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    Some(VideoErrorObject {
        code: v
            .get("code")
            .and_then(|c| c.as_str())
            .map(str::to_string),
        message: v
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_create_fixture() {
        let text = r#"{
            "id": "video_abc",
            "object": "video",
            "created_at": 1710000000,
            "status": "queued",
            "model": "sora-2",
            "progress": 0,
            "seconds": "8",
            "size": "1280x720"
        }"#;
        let v = parse_video_json(text).unwrap();
        assert_eq!(v["id"], "video_abc");
        assert_eq!(v["status"], "queued");
    }

    #[test]
    fn empty_body_is_explicit_eof_hint() {
        let err = parse_video_json("").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("EOF"), "{msg}");
        assert!(msg.contains("claw/v1") || msg.contains("catalog"), "{msg}");
    }
}
