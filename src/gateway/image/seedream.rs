use reqwest::Client;
use serde_json::{json, Value};
use tracing::warn;

use crate::gateway::config::ResolvedImageUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::image::{
    image_to_data_url, join_url, maybe_download_b64, openai_error_message, resolve_model_name,
};
use crate::gateway::image::types::{
    wants_b64, ImageData, ImageEditRequest, ImageGenerateRequest, ImagesResponse,
};

pub async fn generate(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
) -> AppResult<ImagesResponse> {
    call(http, target, req, None).await
}

pub async fn edit(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageEditRequest,
) -> AppResult<ImagesResponse> {
    if req.mask.is_some() {
        warn!("seedream image edit: mask is not supported; ignoring mask");
    }
    let generate_req = ImageGenerateRequest {
        model: req.model.clone(),
        prompt: req.prompt.clone(),
        n: req.n,
        size: req.size.clone(),
        quality: None,
        response_format: req.response_format.clone(),
        user: req.user.clone(),
    };
    let data_url = image_to_data_url(&req.image.bytes, &req.image.mime);
    call(http, target, &generate_req, Some(data_url)).await
}

async fn call(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
    image: Option<String>,
) -> AppResult<ImagesResponse> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image seedream base_url missing".into()))?;
    let url = join_url(base, "images/generations");
    let model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "doubao-seedream-4-0-250828".to_string());

    let mut body = json!({
        "model": model,
        "prompt": req.prompt,
        "stream": false,
        "watermark": false,
    });
    // Prefer disabled sequential generation for single-shot t2i/i2i.
    body["sequential_image_generation"] = json!("disabled");
    if let Some(n) = req.n {
        body["n"] = json!(n);
    }
    if let Some(size) = &req.size {
        body["size"] = json!(size);
    }
    let fmt = req.response_format.as_deref().unwrap_or("url");
    let normalized_fmt = match fmt.trim().to_ascii_lowercase().as_str() {
        "b64_json" | "base64" => "b64_json",
        _ => "url",
    };
    body["response_format"] = json!(normalized_fmt);
    if let Some(img) = image {
        body["image"] = json!(img);
    }

    let mut builder = http.post(&url).json(&body);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("seedream images: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("seedream images body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let parsed = parse_response(&text)?;
    maybe_download_b64(http, parsed, wants_b64(req.response_format.as_deref())).await
}

fn parse_response(text: &str) -> AppResult<ImagesResponse> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| AppError::Upstream(format!("invalid seedream json: {e}")))?;
    let created = v.get("created").and_then(|c| c.as_i64()).unwrap_or(0);
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .map(|item| {
                    let b64 = item
                        .get("b64_json")
                        .or_else(|| item.get("base64"))
                        .and_then(|u| u.as_str())
                        .map(str::to_string);
                    ImageData {
                        url: item
                            .get("url")
                            .and_then(|u| u.as_str())
                            .map(str::to_string),
                        b64_json: b64,
                        revised_prompt: None,
                    }
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
    fn parses_seedream_url_response() {
        let raw = r#"{"created":1,"data":[{"url":"https://img","size":"1024x1024"}]}"#;
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.data[0].url.as_deref(), Some("https://img"));
    }
}
