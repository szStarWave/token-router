//! MiniMax H3 video generation (async create + poll + cancel/delete).
//! Protocol mirrors herdsman `pkg/videogen/providers/minimax_h3.go`.

use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::types::{
    ImageRef, VideoCreateRequest, VideoErrorObject, VideoJob, now_unix,
};
use crate::gateway::video::{
    join_url, openai_error_message, parse_wxh, resolve_model_name, seconds_to_u32,
};

const DEFAULT_MODEL: &str = "MiniMax-H3";
const CREATE_PATH: &str = "v2/video_generation";
const QUERY_PREFIX: &str = "v2/query/video_generation/";
const UPLOAD_PATH: &str = "v1/files/upload";
const USER_AGENT: &str = "token-router-videogen/1.0";
const MAX_REFERENCE_IMAGES: usize = 9;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

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
        .ok_or_else(|| AppError::Unavailable("video minimax base_url missing".into()))?;
    let api_key = target.api_key.as_deref();
    let model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let duration = clamp_duration(seconds_to_u32(req.seconds.as_deref()));

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
    let resolution = normalize_resolution(req.resolution.as_deref());
    let ratio = normalize_ratio(size_to_ratio(req.size.as_deref()), has_media);

    let mut content = vec![json!({
        "type": "text",
        "text": req.prompt,
    })];

    if let Some(reference) = &req.input_reference {
        let url = materialize_image(http, base, api_key, reference).await?;
        content.push(json!({
            "type": "image_url",
            "role": "first_frame",
            "image_url": { "url": url },
        }));
    }
    if let Some(last) = &req.last_frame {
        let url = materialize_image(http, base, api_key, last).await?;
        content.push(json!({
            "type": "image_url",
            "role": "last_frame",
            "image_url": { "url": url },
        }));
    }
    for reference in refs {
        let url = materialize_image(http, base, api_key, reference).await?;
        content.push(json!({
            "type": "image_url",
            "role": "reference_image",
            "image_url": { "url": url },
        }));
    }

    let mut body = json!({
        "model": model,
        "content": content,
        "resolution": resolution,
        "duration": duration,
        "ratio": ratio,
    });
    if req.watermark == Some(true) {
        body["aigc_watermark"] = json!(true);
    }

    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("minimax serialize: {e}")))?;
    if body_bytes.len() > MAX_BODY_BYTES {
        return Err(AppError::BadRequest("request body exceeds 64MB".into()));
    }

    let url = join_url(base, CREATE_PATH);
    let mut builder = http
        .post(&url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/json")
        .body(body_bytes);
    if let Some(key) = api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("minimax create: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("minimax create body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("minimax create json: {e}")))?;
    if let Some(msg) = openapi_or_base_error(&v) {
        return Err(AppError::Upstream(format!("minimax create failed: {msg}")));
    }
    let task_id = extract_task_id(&v)
        .ok_or_else(|| AppError::Upstream(format!("minimax missing task_id: {text}")))?;

    let now = now_unix();
    Ok(VideoJob {
        id: local_id.to_string(),
        provider: "minimax".into(),
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
        .ok_or_else(|| AppError::Unavailable("video minimax base_url missing".into()))?;
    let task_id = job
        .upstream_task_id
        .as_deref()
        .ok_or_else(|| AppError::Upstream("minimax missing task id".into()))?;
    let url = join_url(base, &format!("{QUERY_PREFIX}{task_id}"));
    let mut builder = http
        .get(&url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("minimax query: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("minimax query body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("minimax query json: {e}")))?;
    if let Some(msg) = openapi_or_base_error(&v) {
        return Err(AppError::Upstream(format!("minimax query failed: {msg}")));
    }

    let task_status = extract_status(&v).unwrap_or("queued");
    let mut out = job.clone();
    match task_status.to_ascii_lowercase().as_str() {
        "queued" | "pending" | "preparing" => {
            out.status = "queued".into();
            out.progress = out.progress.max(5);
        }
        "running" | "processing" | "in_progress" | "generating" => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(40).min(90);
        }
        "success" | "succeeded" | "finished" | "complete" | "completed" => {
            out.status = "completed".into();
            out.progress = 100;
            out.result_url = extract_video_url(&v);
            if out.result_url.is_none() {
                if let Some(file_id) = extract_file_id(&v) {
                    out.result_url =
                        Some(join_url(base, &format!("v1/files/retrieve?file_id={file_id}")));
                }
            }
            if out.result_url.is_none() {
                out.status = "failed".into();
                out.error = Some(VideoErrorObject {
                    code: Some("missing_video_url".into()),
                    message: Some("minimax succeeded but no video_url".into()),
                });
            }
        }
        "failed" | "fail" | "error" | "expired" => {
            out.status = "failed".into();
            out.error = Some(VideoErrorObject {
                code: Some(task_status.to_string()),
                message: Some(extract_error_message(&v).unwrap_or_else(|| {
                    "minimax video task failed".into()
                })),
            });
        }
        other if other.starts_with("cancel") => {
            out.status = "cancelled".into();
            out.error = Some(VideoErrorObject {
                code: Some(other.to_string()),
                message: Some("minimax video task cancelled".into()),
            });
        }
        _ => {
            out.status = "in_progress".into();
            out.progress = out.progress.max(20);
        }
    }
    out.touch();
    Ok(out)
}

/// Cancel a queued MiniMax task. Upstream cannot cancel `running`.
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
    if matches!(
        status.as_str(),
        "in_progress" | "running" | "processing" | "generating"
    ) {
        return Err(AppError::BadRequest(
            "minimax cannot cancel a running video task".into(),
        ));
    }
    delete_upstream_task(http, target, job).await
}

/// Delete or cancel upstream task record (best-effort).
pub async fn delete_upstream_task(
    http: &Client,
    target: &ResolvedVideoUpstream,
    job: &VideoJob,
) -> AppResult<()> {
    let Some(task_id) = job.upstream_task_id.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video minimax base_url missing".into()))?;
    let url = join_url(base, &format!("{CREATE_PATH}/{task_id}"));
    let mut builder = http
        .delete(&url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("minimax delete: {e}")))?;
    let status = resp.status();
    // 404 is fine (already gone).
    if status.is_success() || status.as_u16() == 404 {
        return Ok(());
    }
    let text = resp
        .text()
        .await
        .unwrap_or_default();
    Err(AppError::Upstream(openai_error_message(status, &text)))
}

fn clamp_duration(secs: u32) -> u32 {
    secs.clamp(4, 15)
}

/// Prefer explicit inbound `resolution`; default `768P`. Never use openai 720P/1080P labels.
pub(crate) fn normalize_resolution(resolution: Option<&str>) -> String {
    let lower = resolution
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .replace('_', "")
        .replace(' ', "");
    match lower.as_str() {
        "2k" | "1080p" | "1080" | "2160p" | "4k" | "high" => "2K".into(),
        "768p" | "768" | "480p" | "720p" | "low" | "medium" | "auto" | "" => "768P".into(),
        _ if resolution
            .map(|r| r.eq_ignore_ascii_case("768P"))
            .unwrap_or(false) =>
        {
            "768P".into()
        }
        _ if resolution
            .map(|r| r.eq_ignore_ascii_case("2K"))
            .unwrap_or(false) =>
        {
            "2K".into()
        }
        _ => "768P".into(),
    }
}

pub(crate) fn size_to_ratio(size: Option<&str>) -> String {
    let (w, h) = parse_wxh(size);
    let a = w as f32 / h as f32;
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

pub(crate) fn normalize_ratio(ratio: String, has_media: bool) -> String {
    if has_media {
        return "adaptive".into();
    }
    match ratio.as_str() {
        "21:9" | "16:9" | "4:3" | "1:1" | "3:4" | "9:16" => ratio,
        _ => "16:9".into(),
    }
}

/// Whether a URL/data needs MiniMax file upload before create.
pub(crate) fn needs_upload(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("mm_file://") {
        return false;
    }
    if trimmed.starts_with("data:") {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    !(lower.starts_with("https://") || lower.starts_with("http://"))
}

async fn materialize_image(
    http: &Client,
    base: &str,
    api_key: Option<&str>,
    reference: &ImageRef,
) -> AppResult<String> {
    match reference {
        ImageRef::Url(url) if !needs_upload(url) => Ok(url.clone()),
        ImageRef::Url(url) if url.trim().starts_with("data:") => {
            let (mime, bytes, filename) = decode_data_uri(url)?;
            upload_bytes(http, base, api_key, &bytes, &mime, &filename).await
        }
        ImageRef::Url(url) => {
            // Non-public / local-style URL: fetch then upload.
            let resp = http
                .get(url)
                .send()
                .await
                .map_err(|e| AppError::Upstream(format!("minimax fetch media: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(AppError::Upstream(openai_error_message(status, &text)));
            }
            let mime = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::Upstream(format!("minimax fetch media body: {e}")))?;
            let filename = guess_filename_from_url(url, &mime);
            upload_bytes(http, base, api_key, &bytes, &mime, &filename).await
        }
        ImageRef::Bytes {
            bytes,
            mime,
            filename,
        } => upload_bytes(http, base, api_key, bytes, mime, filename).await,
    }
}

fn decode_data_uri(url: &str) -> AppResult<(String, Vec<u8>, String)> {
    let rest = url
        .strip_prefix("data:")
        .ok_or_else(|| AppError::BadRequest("invalid data URI".into()))?;
    let (meta, data) = rest
        .split_once(',')
        .ok_or_else(|| AppError::BadRequest("invalid data URI".into()))?;
    let mime = meta
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string();
    let is_base64 = meta.to_ascii_lowercase().contains("base64");
    let bytes = if is_base64 {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data.trim())
            .map_err(|e| AppError::BadRequest(format!("invalid data URI base64: {e}")))?
    } else {
        data.as_bytes().to_vec()
    };
    let filename = match mime.as_str() {
        "image/png" => "upload.png",
        "image/jpeg" | "image/jpg" => "upload.jpg",
        "image/webp" => "upload.webp",
        _ => "upload.bin",
    }
    .to_string();
    Ok((mime, bytes, filename))
}

fn guess_filename_from_url(url: &str, mime: &str) -> String {
    if let Some(name) = url
        .rsplit('/')
        .next()
        .map(|s| s.split('?').next().unwrap_or(s))
        .filter(|s| !s.is_empty() && s.contains('.'))
    {
        return name.to_string();
    }
    match mime {
        "image/png" => "upload.png".into(),
        "image/jpeg" | "image/jpg" => "upload.jpg".into(),
        "image/webp" => "upload.webp".into(),
        _ => "upload.bin".into(),
    }
}

async fn upload_bytes(
    http: &Client,
    base: &str,
    api_key: Option<&str>,
    bytes: &[u8],
    mime: &str,
    filename: &str,
) -> AppResult<String> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(AppError::BadRequest("upload exceeds 64MB".into()));
    }
    let part = reqwest::multipart::Part::bytes(bytes.to_vec())
        .file_name(filename.to_string())
        .mime_str(mime)
        .unwrap_or_else(|_| {
            reqwest::multipart::Part::bytes(bytes.to_vec()).file_name(filename.to_string())
        });
    let form = reqwest::multipart::Form::new()
        .text("purpose", "video_generation_input")
        .part("file", part);
    let url = join_url(base, UPLOAD_PATH);
    let mut builder = http
        .post(&url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .multipart(form);
    if let Some(key) = api_key {
        builder = builder.bearer_auth(key);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("minimax upload: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("minimax upload body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("minimax upload json: {e}")))?;
    if let Some(msg) = openapi_or_base_error(&v) {
        return Err(AppError::Upstream(format!("minimax upload failed: {msg}")));
    }
    let file_id = extract_upload_file_id(&v)
        .ok_or_else(|| AppError::Upstream(format!("minimax upload missing file_id: {text}")))?;
    Ok(format!("mm_file://{file_id}"))
}

fn extract_upload_file_id(v: &Value) -> Option<String> {
    v.get("file_id")
        .and_then(|t| t.as_str())
        .or_else(|| v.get("id").and_then(|t| t.as_str()))
        .or_else(|| v.pointer("/file/file_id").and_then(|t| t.as_str()))
        .or_else(|| v.pointer("/file/id").and_then(|t| t.as_str()))
        .map(str::to_string)
}

fn extract_task_id(v: &Value) -> Option<String> {
    v.get("task_id")
        .and_then(|t| t.as_str())
        .or_else(|| v.pointer("/data/task_id").and_then(|t| t.as_str()))
        .or_else(|| v.pointer("/task/id").and_then(|t| t.as_str()))
        .map(str::to_string)
}

fn extract_status(v: &Value) -> Option<&str> {
    v.pointer("/task/status")
        .and_then(|s| s.as_str())
        .or_else(|| v.get("status").and_then(|s| s.as_str()))
        .or_else(|| v.get("task_status").and_then(|s| s.as_str()))
}

fn extract_video_url(v: &Value) -> Option<String> {
    let task = v.get("task").unwrap_or(v);
    task.pointer("/content/video_url")
        .or_else(|| task.pointer("/content/url"))
        .or_else(|| task.pointer("/content/download_url"))
        .or_else(|| v.pointer("/result/content/video_url"))
        .or_else(|| task.get("video_url"))
        .and_then(|u| u.as_str())
        .map(str::to_string)
}

fn extract_file_id(v: &Value) -> Option<String> {
    let task = v.get("task").unwrap_or(v);
    task.pointer("/content/file_id")
        .or_else(|| task.get("file_id"))
        .or_else(|| v.get("file_id"))
        .or_else(|| v.pointer("/result/file_id"))
        .and_then(|u| u.as_str())
        .map(str::to_string)
}

fn extract_error_message(v: &Value) -> Option<String> {
    v.pointer("/task/error/message")
        .or_else(|| v.pointer("/error/message"))
        .or_else(|| v.get("message"))
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn openapi_or_base_error(v: &Value) -> Option<String> {
    if let Some(msg) = v.pointer("/error/message").and_then(|m| m.as_str()) {
        if !msg.is_empty() {
            return Some(msg.to_string());
        }
    }
    if let Some(br) = v.get("base_resp") {
        let code = br
            .get("status_code")
            .and_then(|c| c.as_i64())
            .unwrap_or(0);
        if code != 0 {
            let msg = br
                .get("status_msg")
                .or_else(|| br.get("status_message"))
                .and_then(|m| m.as_str())
                .unwrap_or("minimax error");
            return Some(format!("{msg} (status_code={code})"));
        }
    }
    None
}

/// Build MiniMax create JSON for unit tests (no upload / network).
#[cfg(test)]
pub(crate) fn build_create_body_for_test(
    req: &VideoCreateRequest,
    first_url: Option<&str>,
    last_url: Option<&str>,
    ref_urls: &[&str],
) -> AppResult<Value> {
    let has_first = first_url.is_some();
    let has_last = last_url.is_some();
    let has_reference = !ref_urls.is_empty();
    if (has_first || has_last) && has_reference {
        return Err(AppError::BadRequest(
            "first/last frame cannot be mixed with reference media".into(),
        ));
    }
    let has_media = has_first || has_last || has_reference;
    let mut content = vec![json!({ "type": "text", "text": req.prompt })];
    if let Some(url) = first_url {
        content.push(json!({
            "type": "image_url",
            "role": "first_frame",
            "image_url": { "url": url },
        }));
    }
    if let Some(url) = last_url {
        content.push(json!({
            "type": "image_url",
            "role": "last_frame",
            "image_url": { "url": url },
        }));
    }
    for url in ref_urls {
        content.push(json!({
            "type": "image_url",
            "role": "reference_image",
            "image_url": { "url": url },
        }));
    }
    let mut body = json!({
        "model": req.model.as_deref().unwrap_or(DEFAULT_MODEL),
        "content": content,
        "resolution": normalize_resolution(req.resolution.as_deref()),
        "duration": clamp_duration(seconds_to_u32(req.seconds.as_deref())),
        "ratio": normalize_ratio(size_to_ratio(req.size.as_deref()), has_media),
    });
    if req.watermark == Some(true) {
        body["aigc_watermark"] = json!(true);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_prefers_inbound_over_size() {
        // size would be 2K by pixel count; inbound 768P wins.
        assert_eq!(normalize_resolution(Some("768P")), "768P");
        assert_eq!(normalize_resolution(Some("2K")), "2K");
        assert_eq!(normalize_resolution(Some("1080P")), "2K");
        assert_eq!(normalize_resolution(None), "768P");
    }

    #[test]
    fn size_maps_to_ratio_only() {
        assert_eq!(size_to_ratio(Some("1280x720")), "16:9");
        assert_eq!(size_to_ratio(Some("720x1280")), "9:16");
        assert_eq!(size_to_ratio(Some("1024x1024")), "1:1");
        assert_eq!(size_to_ratio(Some("2560x1080")), "21:9");
    }

    #[test]
    fn ratio_adaptive_with_media() {
        assert_eq!(normalize_ratio("16:9".into(), true), "adaptive");
        assert_eq!(normalize_ratio("16:9".into(), false), "16:9");
    }

    #[test]
    fn duration_clamped() {
        assert_eq!(clamp_duration(1), 4);
        assert_eq!(clamp_duration(5), 5);
        assert_eq!(clamp_duration(99), 15);
    }

    #[test]
    fn needs_upload_detects_data_uri() {
        assert!(needs_upload("data:image/png;base64,AAAA"));
        assert!(!needs_upload("https://cdn.example.com/a.png"));
        assert!(!needs_upload("http://cdn.example.com/a.png"));
        assert!(!needs_upload("mm_file://file_abc"));
        assert!(needs_upload("file:///tmp/a.png"));
    }

    #[test]
    fn build_body_watermark_and_frames() {
        let req = VideoCreateRequest {
            prompt: "a cat".into(),
            model: Some("MiniMax-H3".into()),
            seconds: Some("6".into()),
            size: Some("1280x720".into()),
            input_reference: None,
            resolution: Some("2K".into()),
            last_frame: None,
            reference_images: vec![],
            watermark: Some(true),
        };
        let body = build_create_body_for_test(
            &req,
            Some("https://a/first.png"),
            Some("https://a/last.png"),
            &[],
        )
        .unwrap();
        assert_eq!(body["resolution"], json!("2K"));
        assert_eq!(body["ratio"], json!("adaptive"));
        assert_eq!(body["duration"], json!(6));
        assert_eq!(body["aigc_watermark"], json!(true));
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[1]["role"], json!("first_frame"));
        assert_eq!(content[2]["role"], json!("last_frame"));
    }

    #[test]
    fn rejects_frame_and_reference_mix() {
        let req = VideoCreateRequest {
            prompt: "x".into(),
            model: None,
            seconds: None,
            size: None,
            input_reference: None,
            resolution: None,
            last_frame: None,
            reference_images: vec![],
            watermark: None,
        };
        let err = build_create_body_for_test(&req, Some("https://a/f.png"), None, &["https://a/r.png"])
            .unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(msg.contains("cannot be mixed")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn extracts_task_and_result() {
        let create: Value = serde_json::from_str(r#"{"task_id":"t1"}"#).unwrap();
        assert_eq!(extract_task_id(&create).as_deref(), Some("t1"));

        let query: Value = serde_json::from_str(
            r#"{
            "task": {
                "status": "success",
                "content": { "video_url": "https://example.com/a.mp4" }
            }
        }"#,
        )
        .unwrap();
        assert_eq!(extract_status(&query), Some("success"));
        assert_eq!(
            extract_video_url(&query).as_deref(),
            Some("https://example.com/a.mp4")
        );
    }

    #[test]
    fn decode_data_uri_png() {
        let (mime, bytes, name) =
            decode_data_uri("data:image/png;base64,QUFBQQ==").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"AAAA");
        assert_eq!(name, "upload.png");
    }
}
