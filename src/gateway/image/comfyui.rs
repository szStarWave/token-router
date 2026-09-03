use std::fs;
use std::path::Path;
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};
use tokio::time::sleep;
use tracing::warn;

use crate::gateway::config::ResolvedImageUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::image::{join_url, parse_wxh, resolve_model_name};
use crate::gateway::image::types::{
    ImageData, ImageEditRequest, ImageGenerateRequest, ImagesResponse,
};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_POLLS: u32 = 360;

pub async fn generate(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
) -> AppResult<ImagesResponse> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image comfyui base_url missing".into()))?;
    let mut workflow = load_workflow(target.workflow_file.as_deref(), default_t2i_workflow())?;
    inject_t2i(&mut workflow, target, req)?;
    run_workflow(http, base, workflow).await
}

pub async fn edit(
    http: &Client,
    target: &ResolvedImageUpstream,
    req: &ImageEditRequest,
) -> AppResult<ImagesResponse> {
    if req.mask.is_some() {
        warn!("comfyui image edit: mask ignored in v1 (whole-image img2img)");
    }
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("image comfyui base_url missing".into()))?;

    let filename = upload_image(http, base, &req.image.bytes, &req.image.filename).await?;
    let mut workflow = load_workflow(target.workflow_file_i2i.as_deref(), default_i2i_workflow())?;
    inject_i2i(&mut workflow, target, req, &filename)?;
    run_workflow(http, base, workflow).await
}

fn load_workflow(path: Option<&str>, builtin: Value) -> AppResult<Value> {
    let Some(path) = path.map(str::trim).filter(|p| !p.is_empty()) else {
        return Ok(builtin);
    };
    let raw = fs::read_to_string(Path::new(path))
        .map_err(|e| AppError::BadRequest(format!("comfyui workflow_file `{path}`: {e}")))?;
    serde_json::from_str(&raw)
        .map_err(|e| AppError::BadRequest(format!("invalid comfyui workflow json: {e}")))
}

fn inject_t2i(
    workflow: &mut Value,
    target: &ResolvedImageUpstream,
    req: &ImageGenerateRequest,
) -> AppResult<()> {
    let ckpt = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "v1-5-pruned-emaonly.safetensors".to_string());
    let (w, h) = parse_wxh(req.size.as_deref());
    let n = req.n.unwrap_or(1).max(1);

    set_input(workflow, "4", "ckpt_name", json!(ckpt))?;
    set_input(workflow, "6", "text", json!(req.prompt))?;
    set_input(workflow, "5", "width", json!(w))?;
    set_input(workflow, "5", "height", json!(h))?;
    set_input(workflow, "5", "batch_size", json!(n))?;
    set_input(
        workflow,
        "3",
        "seed",
        json!(rand_seed()),
    )?;
    Ok(())
}

fn inject_i2i(
    workflow: &mut Value,
    target: &ResolvedImageUpstream,
    req: &ImageEditRequest,
    uploaded_name: &str,
) -> AppResult<()> {
    let ckpt = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "v1-5-pruned-emaonly.safetensors".to_string());
    let (w, h) = parse_wxh(req.size.as_deref());
    let n = req.n.unwrap_or(1).max(1);

    set_input(workflow, "4", "ckpt_name", json!(ckpt))?;
    set_input(workflow, "6", "text", json!(req.prompt))?;
    set_input(workflow, "10", "image", json!(uploaded_name))?;
    // Keep latent size aligned when EmptyLatent not used; VAEEncode uses image size.
    let _ = (w, h, n);
    set_input(workflow, "3", "seed", json!(rand_seed()))?;
    set_input(workflow, "3", "denoise", json!(0.65))?;
    Ok(())
}

fn set_input(workflow: &mut Value, node_id: &str, key: &str, value: Value) -> AppResult<()> {
    let Some(node) = workflow.get_mut(node_id) else {
        // Custom workflows may use different ids; best-effort skip.
        return Ok(());
    };
    let inputs = node
        .get_mut("inputs")
        .ok_or_else(|| AppError::BadRequest(format!("comfyui node {node_id} missing inputs")))?;
    inputs[key] = value;
    Ok(())
}

fn rand_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42)
}

async fn upload_image(
    http: &Client,
    base: &str,
    bytes: &[u8],
    filename: &str,
) -> AppResult<String> {
    let url = join_url(base, "upload/image");
    let part = reqwest::multipart::Part::bytes(bytes.to_vec())
        .file_name(filename.to_string())
        .mime_str("application/octet-stream")
        .unwrap_or_else(|_| reqwest::multipart::Part::bytes(bytes.to_vec()));
    let form = reqwest::multipart::Form::new()
        .part("image", part)
        .text("overwrite", "true");
    let resp = http
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui upload: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui upload body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "comfyui upload HTTP {status}: {text}"
        )));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("comfyui upload json: {e}")))?;
    v.get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Upstream(format!("comfyui upload missing name: {text}")))
}

async fn run_workflow(http: &Client, base: &str, workflow: Value) -> AppResult<ImagesResponse> {
    let client_id = uuid::Uuid::new_v4().to_string();
    let prompt_body = json!({
        "prompt": workflow,
        "client_id": client_id,
    });
    let url = join_url(base, "prompt");
    let resp = http
        .post(&url)
        .json(&prompt_body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui prompt: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui prompt body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "comfyui prompt HTTP {status}: {text}"
        )));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("comfyui prompt json: {e}")))?;
    let prompt_id = v
        .get("prompt_id")
        .and_then(|p| p.as_str())
        .ok_or_else(|| AppError::Upstream(format!("comfyui missing prompt_id: {text}")))?
        .to_string();

    let history = wait_history(http, base, &prompt_id).await?;
    let images = extract_images_from_history(&history)?;
    let mut data = Vec::new();
    for (filename, subfolder, img_type) in images {
        let bytes = fetch_view(http, base, &filename, &subfolder, &img_type).await?;
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        data.push(ImageData {
            url: None,
            b64_json: Some(b64),
            revised_prompt: None,
        });
    }
    if data.is_empty() {
        return Err(AppError::Upstream(
            "comfyui finished but produced no images".into(),
        ));
    }
    Ok(ImagesResponse::now_with_data(data))
}

async fn wait_history(http: &Client, base: &str, prompt_id: &str) -> AppResult<Value> {
    let url = join_url(base, &format!("history/{prompt_id}"));
    for _ in 0..MAX_POLLS {
        let resp = http
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("comfyui history: {e}")))?;
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Upstream(format!("comfyui history body: {e}")))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(json!({}));
        if let Some(entry) = v.get(prompt_id) {
            if entry.get("outputs").is_some() {
                return Ok(entry.clone());
            }
        }
        sleep(POLL_INTERVAL).await;
    }
    Err(AppError::Upstream(
        "comfyui timed out waiting for history".into(),
    ))
}

fn extract_images_from_history(entry: &Value) -> AppResult<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    let Some(outputs) = entry.get("outputs").and_then(|o| o.as_object()) else {
        return Ok(out);
    };
    for (_node, output) in outputs {
        if let Some(images) = output.get("images").and_then(|i| i.as_array()) {
            for img in images {
                let filename = img
                    .get("filename")
                    .and_then(|f| f.as_str())
                    .unwrap_or("")
                    .to_string();
                if filename.is_empty() {
                    continue;
                }
                let subfolder = img
                    .get("subfolder")
                    .and_then(|f| f.as_str())
                    .unwrap_or("")
                    .to_string();
                let img_type = img
                    .get("type")
                    .and_then(|f| f.as_str())
                    .unwrap_or("output")
                    .to_string();
                out.push((filename, subfolder, img_type));
            }
        }
    }
    Ok(out)
}

async fn fetch_view(
    http: &Client,
    base: &str,
    filename: &str,
    subfolder: &str,
    img_type: &str,
) -> AppResult<Vec<u8>> {
    let url = format!(
        "{}?filename={}&subfolder={}&type={}",
        join_url(base, "view"),
        urlencoding_filename(filename),
        urlencoding_filename(subfolder),
        urlencoding_filename(img_type)
    );
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui view: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "comfyui view HTTP {status}"
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui view body: {e}")))?;
    Ok(bytes.to_vec())
}

fn urlencoding_filename(s: &str) -> String {
    // Minimal encoding for query values.
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

fn default_t2i_workflow() -> Value {
    json!({
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "cfg": 7.0,
                "denoise": 1.0,
                "latent_image": ["5", 0],
                "model": ["4", 0],
                "negative": ["7", 0],
                "positive": ["6", 0],
                "sampler_name": "euler",
                "scheduler": "normal",
                "seed": 0,
                "steps": 20
            }
        },
        "4": {
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": "v1-5-pruned-emaonly.safetensors" }
        },
        "5": {
            "class_type": "EmptyLatentImage",
            "inputs": { "batch_size": 1, "height": 512, "width": 512 }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "clip": ["4", 1], "text": "" }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "clip": ["4", 1], "text": "bad quality, blurry" }
        },
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["3", 0], "vae": ["4", 2] }
        },
        "9": {
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": "token_router", "images": ["8", 0] }
        }
    })
}

fn default_i2i_workflow() -> Value {
    json!({
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "cfg": 7.0,
                "denoise": 0.65,
                "latent_image": ["11", 0],
                "model": ["4", 0],
                "negative": ["7", 0],
                "positive": ["6", 0],
                "sampler_name": "euler",
                "scheduler": "normal",
                "seed": 0,
                "steps": 20
            }
        },
        "4": {
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": "v1-5-pruned-emaonly.safetensors" }
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": { "clip": ["4", 1], "text": "" }
        },
        "7": {
            "class_type": "CLIPTextEncode",
            "inputs": { "clip": ["4", 1], "text": "bad quality, blurry" }
        },
        "8": {
            "class_type": "VAEDecode",
            "inputs": { "samples": ["3", 0], "vae": ["4", 2] }
        },
        "9": {
            "class_type": "SaveImage",
            "inputs": { "filename_prefix": "token_router_i2i", "images": ["8", 0] }
        },
        "10": {
            "class_type": "LoadImage",
            "inputs": { "image": "input.png" }
        },
        "11": {
            "class_type": "VAEEncode",
            "inputs": { "pixels": ["10", 0], "vae": ["4", 2] }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::config::ResolvedImageUpstream;

    #[test]
    fn inject_t2i_sets_prompt_and_size() {
        let mut wf = default_t2i_workflow();
        let target = ResolvedImageUpstream {
            provider: "comfyui".into(),
            base_url: Some("http://127.0.0.1:8188".into()),
            api_key: None,
            model: Some("model.safetensors".into()),
            upstream_model: None,
            workflow_file: None,
            workflow_file_i2i: None,
        };
        let req = ImageGenerateRequest {
            model: None,
            prompt: "a cat".into(),
            n: Some(1),
            size: Some("768x512".into()),
            quality: None,
            response_format: None,
            user: None,
        };
        inject_t2i(&mut wf, &target, &req).unwrap();
        assert_eq!(wf["6"]["inputs"]["text"], json!("a cat"));
        assert_eq!(wf["5"]["inputs"]["width"], json!(768));
        assert_eq!(wf["4"]["inputs"]["ckpt_name"], json!("model.safetensors"));
    }

    #[test]
    fn inject_i2i_sets_uploaded_name() {
        let mut wf = default_i2i_workflow();
        let target = ResolvedImageUpstream {
            provider: "comfyui".into(),
            base_url: Some("http://127.0.0.1:8188".into()),
            api_key: None,
            model: None,
            upstream_model: None,
            workflow_file: None,
            workflow_file_i2i: None,
        };
        let req = ImageEditRequest {
            model: None,
            prompt: "make it night".into(),
            image: crate::gateway::image::types::ImageBytes {
                bytes: vec![1, 2, 3],
                mime: "image/png".into(),
                filename: "x.png".into(),
            },
            mask: None,
            n: Some(1),
            size: None,
            response_format: None,
            user: None,
        };
        inject_i2i(&mut wf, &target, &req, "uploaded.png").unwrap();
        assert_eq!(wf["10"]["inputs"]["image"], json!("uploaded.png"));
        assert_eq!(wf["6"]["inputs"]["text"], json!("make it night"));
    }
}
