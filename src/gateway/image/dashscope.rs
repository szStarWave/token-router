use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};
use tokio::time::sleep;

use crate::gateway::config::ResolvedImageUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::image::{
    image_to_data_url, join_url, maybe_download_b64, openai_error_message, resolve_model_name,
    size_to_dashscope,
};
use crate::gateway::image::types::{
    wants_b64, ImageData, ImageEditRequest, ImageGenerateRequest, ImagesResponse,
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_POLLS: u32 = 90;

pub async fn generate(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
) -> AppResult<ImagesResponse> {
    let model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "wan2.6-t2i".to_string());
    if is_wan26(&model) {
        generate_sync_wan26(http, target, req, &model).await
    } else {
        generate_async_legacy(http, target, req, &model).await
    }
}

pub async fn edit(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageEditRequest,
) -> AppResult<ImagesResponse> {
    let model = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "wanx2.1-imageedit".to_string());
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image dashscope base_url missing".into()))?;

    let data_url = image_to_data_url(&req.image.bytes, &req.image.mime);
    // Image edit / i2i via multimodal-style or image-generation async APIs.
    let url = join_url(base, "services/aigc/image2image/image-synthesis");
    let body = json!({
        "model": model,
        "input": {
            "prompt": req.prompt,
            "images": [data_url],
        },
        "parameters": {
            "n": req.n.unwrap_or(1),
            "size": size_to_dashscope(req.size.as_deref()),
        }
    });

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
        .map_err(|e| AppError::Upstream(format!("dashscope i2i: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope i2i body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let task_id = extract_task_id(&text)?;
    let result = poll_task(http, base, target.api_key.as_deref(), &task_id).await?;
    let parsed = parse_task_result(&result)?;
    maybe_download_b64(http, parsed, wants_b64(req.response_format.as_deref())).await
}

async fn generate_sync_wan26(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
    model: &str,
) -> AppResult<ImagesResponse> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image dashscope base_url missing".into()))?;
    let url = join_url(base, "services/aigc/multimodal-generation/generation");
    let body = json!({
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": [{ "text": req.prompt }]
            }]
        },
        "parameters": {
            "prompt_extend": true,
            "watermark": false,
            "n": req.n.unwrap_or(1),
            "size": size_to_dashscope(req.size.as_deref()),
        }
    });

    let mut builder = http.post(&url).json(&body);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope t2i sync: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope t2i sync body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let parsed = parse_sync_wan26(&text)?;
    maybe_download_b64(http, parsed, wants_b64(req.response_format.as_deref())).await
}

async fn generate_async_legacy(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
    model: &str,
) -> AppResult<ImagesResponse> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image dashscope base_url missing".into()))?;
    let url = join_url(base, "services/aigc/text2image/image-synthesis");
    let body = json!({
        "model": model,
        "input": {
            "prompt": req.prompt,
        },
        "parameters": {
            "size": size_to_dashscope(req.size.as_deref()),
            "n": req.n.unwrap_or(1),
            "prompt_extend": true,
            "watermark": false,
        }
    });

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
        .map_err(|e| AppError::Upstream(format!("dashscope t2i async: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("dashscope t2i async body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(openai_error_message(status, &text)));
    }

    let task_id = extract_task_id(&text)?;
    let result = poll_task(http, base, target.api_key.as_deref(), &task_id).await?;
    let parsed = parse_task_result(&result)?;
    maybe_download_b64(http, parsed, wants_b64(req.response_format.as_deref())).await
}

fn is_wan26(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("wan2.6") || m.starts_with("wan2.7")
}

fn extract_task_id(text: &str) -> AppResult<String> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| AppError::Upstream(format!("dashscope task create json: {e}")))?;
    v.pointer("/output/task_id")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Upstream(format!("dashscope missing task_id: {text}")))
}

async fn poll_task(
    http: &Client,
    base: &str,
    api_key: Option<&str>,
    task_id: &str,
) -> AppResult<Value> {
    let url = join_url(base, &format!("tasks/{task_id}"));
    for _ in 0..MAX_POLLS {
        let mut builder = http.get(&url);
        if let Some(key) = api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("dashscope poll: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Upstream(format!("dashscope poll body: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Upstream(openai_error_message(status, &text)));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AppError::Upstream(format!("dashscope poll json: {e}")))?;
        let status_s = v
            .pointer("/output/task_status")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        match status_s {
            "SUCCEEDED" => return Ok(v),
            "FAILED" | "CANCELED" | "UNKNOWN" => {
                let msg = v
                    .pointer("/output/message")
                    .or_else(|| v.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or(status_s);
                return Err(AppError::Upstream(format!("dashscope task {status_s}: {msg}")));
            }
            _ => sleep(POLL_INTERVAL).await,
        }
    }
    Err(AppError::Upstream(
        "dashscope task timed out waiting for result".into(),
    ))
}

fn parse_sync_wan26(text: &str) -> AppResult<ImagesResponse> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| AppError::Upstream(format!("dashscope sync json: {e}")))?;
    let mut data = Vec::new();
    if let Some(choices) = v.pointer("/output/choices").and_then(|c| c.as_array()) {
        for choice in choices {
            if let Some(content) = choice.pointer("/message/content").and_then(|c| c.as_array()) {
                for part in content {
                    if let Some(url) = part.get("image").and_then(|u| u.as_str()) {
                        data.push(ImageData {
                            url: Some(url.to_string()),
                            b64_json: None,
                            revised_prompt: None,
                        });
                    }
                }
            }
        }
    }
    if data.is_empty() {
        return Err(AppError::Upstream(format!(
            "dashscope sync: no images in response: {text}"
        )));
    }
    Ok(ImagesResponse::now_with_data(data))
}

fn parse_task_result(v: &Value) -> AppResult<ImagesResponse> {
    let mut data = Vec::new();

    if let Some(results) = v.pointer("/output/results").and_then(|r| r.as_array()) {
        for item in results {
            let url = item.get("url").and_then(|u| u.as_str()).map(str::to_string);
            let revised = item
                .get("actual_prompt")
                .and_then(|u| u.as_str())
                .map(str::to_string);
            if url.is_some() {
                data.push(ImageData {
                    url,
                    b64_json: None,
                    revised_prompt: revised,
                });
            }
        }
    }

    if data.is_empty() {
        if let Some(choices) = v.pointer("/output/choices").and_then(|c| c.as_array()) {
            for choice in choices {
                if let Some(content) = choice.pointer("/message/content").and_then(|c| c.as_array())
                {
                    for part in content {
                        if let Some(url) = part.get("image").and_then(|u| u.as_str()) {
                            data.push(ImageData {
                                url: Some(url.to_string()),
                                b64_json: None,
                                revised_prompt: None,
                            });
                        }
                    }
                }
            }
        }
    }

    if data.is_empty() {
        return Err(AppError::Upstream(
            "dashscope task succeeded but returned no images".into(),
        ));
    }
    Ok(ImagesResponse::now_with_data(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::image::size_to_dashscope;

    #[test]
    fn size_conversion() {
        assert_eq!(size_to_dashscope(Some("1024x1024")), "1024*1024");
    }

    #[test]
    fn wan26_detection() {
        assert!(is_wan26("wan2.6-t2i"));
        assert!(!is_wan26("wan2.5-t2i-preview"));
    }

    #[test]
    fn parse_sync_images() {
        let raw = r#"{
          "output": {
            "choices": [{
              "message": {
                "content": [{"image": "https://x.png", "type": "image"}]
              }
            }]
          }
        }"#;
        let resp = parse_sync_wan26(raw).unwrap();
        assert_eq!(resp.data[0].url.as_deref(), Some("https://x.png"));
    }

    #[test]
    fn parse_async_results() {
        let v: Value = serde_json::from_str(
            r#"{
              "output": {
                "task_status": "SUCCEEDED",
                "results": [{
                  "url": "https://y.png",
                  "actual_prompt": "revised"
                }]
              }
            }"#,
        )
        .unwrap();
        let resp = parse_task_result(&v).unwrap();
        assert_eq!(resp.data[0].revised_prompt.as_deref(), Some("revised"));
    }
}
