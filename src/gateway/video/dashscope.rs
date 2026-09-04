//! Aliyun Bailian / DashScope video-synthesis adapter.
//! Maps OpenAI Videos create/poll into wan2.6 (`size`) vs wan2.7+ (`resolution`+`ratio`).

use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::types::{
    VideoCreateRequest, VideoErrorObject, VideoJob, VideoObject, now_unix,
};
use crate::gateway::video::{
    image_ref_to_url, join_url, openai_error_message, openai_size_to_dashscope_size,
    resolve_model_name, seconds_to_u32, size_to_resolution_ratio, snap_dashscope_ratio,
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
    let duration = seconds_to_u32(req.seconds.as_deref());

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
        duration,
        first_url.as_deref(),
        last_url.as_deref(),
        req.watermark,
    )?;

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
    Ok(apply_query_to_job(job, &v))
}

/// Build DashScope create JSON (unit-testable, no network).
pub(crate) fn build_create_body(
    model: &str,
    prompt: &str,
    size: Option<&str>,
    duration: u32,
    first_url: Option<&str>,
    last_url: Option<&str>,
    watermark: Option<bool>,
) -> AppResult<Value> {
    let is_i2v = model_is_i2v(model) || first_url.is_some();
    let uses_resolution_ratio = model_uses_resolution_ratio(model);
    let watermark = watermark.unwrap_or(false);

    let mut input = json!({ "prompt": prompt });
    if let Some(url) = first_url {
        if uses_resolution_ratio && last_url.is_some() {
            let mut media = vec![json!({ "type": "first_frame", "url": url })];
            if let Some(last) = last_url {
                media.push(json!({ "type": "last_frame", "url": last }));
            }
            input["media"] = json!(media);
        } else {
            // wan2.6 and earlier I2V: img_url only.
            input["img_url"] = json!(url);
        }
    } else if last_url.is_some() {
        return Err(AppError::BadRequest(
            "dashscope last_frame requires first_frame / input_reference".into(),
        ));
    }

    let mut parameters = json!({
        "duration": duration,
        "prompt_extend": true,
        "watermark": watermark,
    });

    if is_i2v {
        // I2V: resolution tier only (aspect follows input image).
        let (resolution, _) = size_to_resolution_ratio(size);
        let resolution = if resolution == "480P" && !model_allows_480p(model) {
            "720P".to_string()
        } else {
            resolution
        };
        parameters["resolution"] = json!(resolution);
    } else if uses_resolution_ratio {
        let (resolution, ratio) = size_to_resolution_ratio(size);
        parameters["resolution"] = json!(resolution);
        parameters["ratio"] = json!(snap_dashscope_ratio(&ratio));
    } else {
        // wan2.6 T2V and earlier: size as W*H.
        parameters["size"] = json!(openai_size_to_dashscope_size(size));
    }

    Ok(json!({
        "model": model,
        "input": input,
        "parameters": parameters,
    }))
}

fn model_is_i2v(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("-i2v") || lower.contains("_i2v") || lower.ends_with("i2v")
}

fn model_uses_resolution_ratio(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.contains("wan2.7")
        || lower.contains("wan2_7")
        || lower.contains("wan3.0")
        || lower.contains("wan3_0")
        || lower.contains("wan3-")
}

fn model_allows_480p(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    !(lower.contains("wan2.6") || lower.contains("wan2_6") || lower.contains("wan2.7"))
}

/// Fold DashScope query JSON into an OpenAI-shaped `VideoJob`.
pub(crate) fn apply_query_to_job(job: &VideoJob, v: &Value) -> VideoJob {
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
            out.error = None;
        }
        "RUNNING" => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(40).min(90);
            out.error = None;
        }
        "SUCCEEDED" | "SUCCESS" => {
            out.status = "completed".into();
            out.progress = 100;
            out.result_url = extract_video_url(v);
            out.error = None;
            backfill_from_usage(&mut out, v);
            if out.result_url.is_none() {
                out.status = "failed".into();
                out.error = Some(VideoErrorObject {
                    code: Some("missing_video_url".into()),
                    message: Some("dashscope succeeded but no video_url".into()),
                });
            }
        }
        "CANCELED" | "CANCELLED" => {
            out.status = "cancelled".into();
            out.error = Some(VideoErrorObject {
                code: Some("cancelled".into()),
                message: Some(extract_error_message(v).unwrap_or_else(|| {
                    "dashscope video task cancelled".into()
                })),
            });
        }
        "FAILED" | "UNKNOWN" => {
            out.status = "failed".into();
            out.error = Some(VideoErrorObject {
                code: Some(
                    extract_error_code(v).unwrap_or_else(|| task_status.to_string()),
                ),
                message: Some(extract_error_message(v).unwrap_or_else(|| {
                    "dashscope video task failed".into()
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

fn backfill_from_usage(job: &mut VideoJob, v: &Value) {
    if job.seconds.is_none() {
        if let Some(d) = v
            .pointer("/usage/output_video_duration")
            .or_else(|| v.pointer("/usage/duration"))
            .and_then(|d| d.as_u64().or_else(|| d.as_f64().map(|f| f as u64)))
        {
            job.seconds = Some(d.to_string());
        }
    }
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

fn extract_error_message(v: &Value) -> Option<String> {
    v.pointer("/output/message")
        .or_else(|| v.get("message"))
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn extract_error_code(v: &Value) -> Option<String> {
    v.pointer("/output/code")
        .or_else(|| v.get("code"))
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wan26_t2v_uses_size_not_resolution() {
        let body = build_create_body(
            "wan2.6-t2v",
            "a cat",
            Some("1280x720"),
            8,
            None,
            None,
            Some(true),
        )
        .unwrap();
        assert_eq!(body["parameters"]["size"], json!("1280*720"));
        assert!(body["parameters"].get("resolution").is_none());
        assert!(body["parameters"].get("ratio").is_none());
        assert_eq!(body["parameters"]["watermark"], json!(true));
        assert_eq!(body["parameters"]["duration"], json!(8));
    }

    #[test]
    fn wan27_t2v_uses_resolution_and_ratio() {
        let body = build_create_body(
            "wan2.7-t2v",
            "a cat",
            Some("1920x1080"),
            5,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(body["parameters"]["resolution"], json!("1080P"));
        assert_eq!(body["parameters"]["ratio"], json!("16:9"));
        assert!(body["parameters"].get("size").is_none());
        assert_eq!(body["parameters"]["watermark"], json!(false));
    }

    #[test]
    fn wan26_i2v_uses_img_url_and_resolution() {
        let body = build_create_body(
            "wan2.6-i2v",
            "animate",
            Some("1280x720"),
            5,
            Some("https://example.com/a.png"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            body["input"]["img_url"],
            json!("https://example.com/a.png")
        );
        assert!(body["input"].get("media").is_none());
        assert_eq!(body["parameters"]["resolution"], json!("720P"));
        assert!(body["parameters"].get("size").is_none());
        assert!(body["parameters"].get("ratio").is_none());
    }

    #[test]
    fn wan27_i2v_last_frame_uses_media() {
        let body = build_create_body(
            "wan2.7-i2v",
            "animate",
            Some("1280x720"),
            5,
            Some("https://example.com/first.png"),
            Some("https://example.com/last.png"),
            None,
        )
        .unwrap();
        let media = body["input"]["media"].as_array().unwrap();
        assert_eq!(media.len(), 2);
        assert_eq!(media[0]["type"], json!("first_frame"));
        assert_eq!(media[1]["type"], json!("last_frame"));
    }

    #[test]
    fn apply_query_succeeded_openai_shape() {
        let job = VideoJob {
            id: "video_x".into(),
            provider: "dashscope".into(),
            tier: "cloud".into(),
            upstream_task_id: Some("t1".into()),
            status: "queued".into(),
            progress: 0,
            model: "wan2.6-t2v".into(),
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
            "output": {
                "task_id": "t1",
                "task_status": "SUCCEEDED",
                "video_url": "https://example.com/a.mp4"
            },
            "usage": { "duration": 5 }
        }"#,
        )
        .unwrap();
        let out = apply_query_to_job(&job, &v);
        assert_eq!(out.status, "completed");
        assert_eq!(out.progress, 100);
        assert_eq!(out.result_url.as_deref(), Some("https://example.com/a.mp4"));
        let obj = VideoObject::from_job(&out);
        let ser = serde_json::to_value(&obj).unwrap();
        assert_eq!(ser["object"], "video");
        assert_eq!(ser["status"], "completed");
        assert!(ser.get("result_url").is_none());
        assert_eq!(ser["id"], "video_x");
    }

    #[test]
    fn apply_query_cancelled_not_failed() {
        let job = VideoJob {
            id: "video_x".into(),
            provider: "dashscope".into(),
            tier: "cloud".into(),
            upstream_task_id: Some("t1".into()),
            status: "in_progress".into(),
            progress: 40,
            model: "wan2.6-t2v".into(),
            seconds: None,
            size: None,
            prompt: None,
            error: None,
            result_url: None,
            local_path: None,
            created_at: 1,
            updated_at: 1,
        };
        let v: Value = serde_json::from_str(
            r#"{"output":{"task_status":"CANCELED","message":"user cancel"}}"#,
        )
        .unwrap();
        let out = apply_query_to_job(&job, &v);
        assert_eq!(out.status, "cancelled");
        assert_eq!(out.error.as_ref().unwrap().code.as_deref(), Some("cancelled"));
    }

    #[test]
    fn apply_query_failed_error_object() {
        let job = VideoJob {
            id: "video_x".into(),
            provider: "dashscope".into(),
            tier: "cloud".into(),
            upstream_task_id: Some("t1".into()),
            status: "queued".into(),
            progress: 0,
            model: "wan2.6-t2v".into(),
            seconds: None,
            size: None,
            prompt: None,
            error: None,
            result_url: None,
            local_path: None,
            created_at: 1,
            updated_at: 1,
        };
        let v: Value = serde_json::from_str(
            r#"{"output":{"task_status":"FAILED","code":"InvalidParameter","message":"bad size"}}"#,
        )
        .unwrap();
        let out = apply_query_to_job(&job, &v);
        assert_eq!(out.status, "failed");
        assert_eq!(
            out.error.as_ref().unwrap().code.as_deref(),
            Some("InvalidParameter")
        );
        assert_eq!(
            out.error.as_ref().unwrap().message.as_deref(),
            Some("bad size")
        );
    }

    #[test]
    fn extract_task_id_fixture() {
        let text = r#"{"output":{"task_id":"task-123","task_status":"PENDING"}}"#;
        assert_eq!(extract_task_id(text).unwrap(), "task-123");
    }
}
