use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedImageUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::flowy::maybe_normalize_flowy_model;
use crate::gateway::image::{join_url, openai_error_message, resolve_model_name};
use crate::gateway::image::types::{ImageData, ImageEditRequest, ImageGenerateRequest, ImagesResponse};

pub async fn generate(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
) -> AppResult<ImagesResponse> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image openai base_url missing".into()))?;
    let url = join_url(base, "images/generations");
    let model = maybe_normalize_flowy_model(
        base,
        &resolve_model_name(target, req.model.as_deref())
            .unwrap_or_else(|| "gpt-image-1".to_string()),
    );

    let mut body = json!({
        "model": model,
        "prompt": req.prompt,
    });
    if let Some(n) = req.n {
        body["n"] = json!(n);
    }
    if let Some(size) = &req.size {
        body["size"] = json!(size);
    }
    if let Some(quality) = &req.quality {
        body["quality"] = json!(quality);
    }
    if let Some(fmt) = &req.response_format {
        body["response_format"] = json!(fmt);
    }
    if let Some(user) = &req.user {
        body["user"] = json!(user);
    }

    let mut builder = http.post(&url).json(&body);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("openai images: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("openai images body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }
    parse_images_response(&text)
}

pub async fn edit(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageEditRequest,
) -> AppResult<ImagesResponse> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image openai base_url missing".into()))?;
    let url = join_url(base, "images/edits");
    let model = maybe_normalize_flowy_model(
        base,
        &resolve_model_name(target, req.model.as_deref())
            .unwrap_or_else(|| "gpt-image-1".to_string()),
    );

    let mut form = reqwest::multipart::Form::new()
        .text("prompt", req.prompt.clone())
        .text("model", model);

    let image_part = reqwest::multipart::Part::bytes(req.image.bytes.clone())
        .file_name(req.image.filename.clone())
        .mime_str(&req.image.mime)
        .map_err(|e| AppError::BadRequest(format!("invalid image mime: {e}")))?;
    form = form.part("image", image_part);

    if let Some(mask) = &req.mask {
        let mask_part = reqwest::multipart::Part::bytes(mask.bytes.clone())
            .file_name(mask.filename.clone())
            .mime_str(&mask.mime)
            .map_err(|e| AppError::BadRequest(format!("invalid mask mime: {e}")))?;
        form = form.part("mask", mask_part);
    }
    if let Some(n) = req.n {
        form = form.text("n", n.to_string());
    }
    if let Some(size) = &req.size {
        form = form.text("size", size.clone());
    }
    if let Some(fmt) = &req.response_format {
        form = form.text("response_format", fmt.clone());
    }
    if let Some(user) = &req.user {
        form = form.text("user", user.clone());
    }

    let mut builder = http.post(&url).multipart(form);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("openai images edits: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("openai images edits body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }
    parse_images_response(&text)
}

fn parse_images_response(text: &str) -> AppResult<ImagesResponse> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| AppError::Upstream(format!("invalid openai images json: {e}")))?;
    let created = v.get("created").and_then(|c| c.as_i64()).unwrap_or(0);
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .map(|item| ImageData {
                    url: item
                        .get("url")
                        .and_then(|u| u.as_str())
                        .map(str::to_string),
                    b64_json: item
                        .get("b64_json")
                        .and_then(|u| u.as_str())
                        .map(str::to_string),
                    revised_prompt: item
                        .get("revised_prompt")
                        .and_then(|u| u.as_str())
                        .map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ImagesResponse { created, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_style_response() {
        let raw = r#"{"created":123,"data":[{"url":"https://x","revised_prompt":"y"}]}"#;
        let resp = parse_images_response(raw).unwrap();
        assert_eq!(resp.created, 123);
        assert_eq!(resp.data[0].url.as_deref(), Some("https://x"));
        assert_eq!(resp.data[0].revised_prompt.as_deref(), Some("y"));
    }
}
