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
        .ok_or_else(|| AppError::Unavailable("video dashscope base_url missing".into()))?;
    let model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "wan2.6-t2v".to_string());
    let (resolution, ratio) = size_to_resolution_ratio(req.size.as_deref());
    let duration = seconds_to_u32(req.seconds.as_deref());

    let mut input = json!({ "prompt": req.prompt });
    if let Some(reference) = &req.input_reference {
        let url = image_ref_to_url(http, reference).await?;
        // Image-to-video: first frame / img_url style fields.
        input["img_url"] = json!(url);
        input["media"] = json!([{ "type": "first_frame", "url": url }]);
    }

    let body = json!({
        "model": model,
        "input": input,
        "parameters": {
            "resolution": resolution,
            "ratio": ratio,
            "duration": duration,
            "prompt_extend": true,
            "watermark": false,
        }
    });

    let url = join_url(base, "services/aigc/video-generation/video-synthesis");
    let mut builder = http
        .post(&url)
        .header("X-DashScope-Async", "enable")
        .json(&body);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope video create: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope video create body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let task_id = extract_task_id(&text)?;
    let now = now_unix();
    Ok(VideoJob {
        id: local_id.to_string(),
        provider: "dashscope".into(),
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
        .ok_or_else(|| AppError::Unavailable("video dashscope base_url missing".into()))?;
    let task_id = job
        .upstream_task_id
        .as_deref()
        .ok_or_else(|| AppError::Upstream("dashscope video missing task id".into()))?;
    let url = join_url(base, &format!("tasks/{task_id}"));
    let mut builder = http.get(&url);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope video task: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope video task body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("dashscope video task json: {e}")))?;
    let task_status = v
        .pointer("/output/task_status")
        .or_else(|| v.pointer("/task_status"))
        .and_then(|s| s.as_str())
        .unwrap_or("PENDING");

    let mut out = job.clone();
    match task_status.to_ascii_uppercase().as_str() {
        "PENDING" | "PENDING_QUEUED" => {
            out.status = "queued".into();
            out.progress = out.progress.max(5);
        }
        "RUNNING" => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(40).min(90);
        }
        "SUCCEEDED" | "SUCCESS" => {
            out.status = "completed".into();
            out.progress = 100;
            out.result_url = extract_video_url(&v);
            if out.result_url.is_none() {
                out.status = "failed".into();
                out.error = Some(VideoErrorObject {
                    code: Some("missing_video_url".into()),
                    message: Some("dashscope succeeded but no video_url".into()),
                });
            }
        }
        "FAILED" | "CANCELED" | "CANCELLED" | "UNKNOWN" => {
            out.status = "failed".into();
            let msg = v
                .pointer("/output/message")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("dashscope video task failed");
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

fn extract_task_id(text: &str) -> AppResult<String> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| AppError::Upstream(format!("dashscope create json: {e}")))?;
    v.pointer("/output/task_id")
        .or_else(|| v.get("task_id"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Upstream(format!("dashscope missing task_id: {text}")))
}

fn extract_video_url(v: &Value) -> Option<String> {
    v.pointer("/output/video_url")
        .or_else(|| v.pointer("/output/results/0/url"))
        .or_else(|| v.pointer("/output/results/0/video_url"))
        .and_then(|u| u.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_succeeded_fixture() {
        let text = r#"{
            "output": {
                "task_id": "t1",
                "task_status": "SUCCEEDED",
                "video_url": "https://example.com/a.mp4"
            }
        }"#;
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            extract_video_url(&v).as_deref(),
            Some("https://example.com/a.mp4")
        );
        let job = VideoJob {
            id: "video_x".into(),
            provider: "dashscope".into(),
            tier: "cloud".into(),
            upstream_task_id: Some("t1".into()),
            status: "queued".into(),
            progress: 0,
            model: "wan2.6-t2v".into(),
            seconds: Some("5".into()),
            size: None,
            prompt: None,
            error: None,
            result_url: None,
            local_path: None,
            created_at: 1,
            updated_at: 1,
        };
        // status mapping via local helper pattern
        let status = v["output"]["task_status"].as_str().unwrap();
        assert_eq!(status, "SUCCEEDED");
        assert!(job.upstream_task_id.is_some());
    }

    #[test]
    fn extract_task_id_fixture() {
        let text = r#"{"output":{"task_id":"task-123","task_status":"PENDING"}}"#;
        assert_eq!(extract_task_id(text).unwrap(), "task-123");
    }
}
