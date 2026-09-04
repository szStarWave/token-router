use std::fs;
use std::path::Path;

use reqwest::Client;
use serde_json::{json, Value};

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::store::VideoJobStore;
use crate::gateway::video::types::{
    ImageRef, VideoCreateRequest, VideoErrorObject, VideoJob, now_unix,
};
use crate::gateway::video::{join_url, parse_wxh, resolve_model_name, seconds_to_u32};

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
        .ok_or_else(|| AppError::Unavailable("video comfyui base_url missing".into()))?;

    let has_ref = req.input_reference.is_some();
    let mut workflow = if has_ref {
        load_workflow(target.workflow_file_i2v.as_deref(), default_i2v_workflow())?
    } else {
        load_workflow(target.workflow_file.as_deref(), default_t2v_workflow())?
    };

    let uploaded = if let Some(ImageRef::Bytes {
        bytes,
        filename,
        ..
    }) = &req.input_reference
    {
        Some(upload_image(http, base, bytes, filename).await?)
    } else if let Some(ImageRef::Url(url)) = &req.input_reference {
        let bytes = http
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("comfyui fetch image_url: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Upstream(format!("comfyui fetch image_url: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Upstream(format!("comfyui fetch image_url body: {e}")))?;
        Some(upload_image(http, base, &bytes, "input_reference.png").await?)
    } else {
        None
    };

    if let Some(name) = &uploaded {
        inject_i2v(&mut workflow, target, req, name)?;
    } else {
        inject_t2v(&mut workflow, target, req)?;
    }

    let prompt_id = submit_prompt(http, base, workflow).await?;
    let now = now_unix();
    let model = resolve_model_name(target, req.model.as_deref()).unwrap_or_default();
    Ok(VideoJob {
        id: local_id.to_string(),
        provider: "comfyui".into(),
        tier: tier.into(),
        upstream_task_id: Some(prompt_id),
        status: "queued".into(),
        progress: 0,
        model,
        seconds: req
            .seconds
            .clone()
            .or_else(|| Some(seconds_to_u32(None).to_string())),
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
    store: &VideoJobStore,
) -> AppResult<VideoJob> {
    let base = target
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Unavailable("video comfyui base_url missing".into()))?;
    let prompt_id = job
        .upstream_task_id
        .as_deref()
        .ok_or_else(|| AppError::Upstream("comfyui missing prompt_id".into()))?;

    let url = join_url(base, &format!("history/{prompt_id}"));
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
    let Some(entry) = v.get(prompt_id) else {
        let mut out = job.clone();
        out.status = "in_progress".into();
        out.progress = out.progress.max(10).min(90);
        out.touch();
        return Ok(out);
    };

    if entry.get("outputs").is_none() {
        let mut out = job.clone();
        out.status = "in_progress".into();
        out.progress = out.progress.max(30).min(90);
        out.touch();
        return Ok(out);
    }

    let media = extract_media_from_history(entry)?;
    if media.is_empty() {
        let mut out = job.clone();
        out.status = "failed".into();
        out.error = Some(VideoErrorObject {
            code: Some("no_video".into()),
            message: Some(
                "comfyui finished but produced no video; set workflow_file for a video workflow"
                    .into(),
            ),
        });
        out.touch();
        return Ok(out);
    }

    let (filename, subfolder, media_type) = &media[0];
    let bytes = fetch_view(http, base, filename, subfolder, media_type).await?;
    let path = store.local_file_path(&job.id);
    fs::write(&path, &bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("write video file: {e}")))?;

    let mut out = job.clone();
    out.status = "completed".into();
    out.progress = 100;
    out.local_path = Some(path.display().to_string());
    out.touch();
    Ok(out)
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

fn inject_t2v(
    workflow: &mut Value,
    target: &ResolvedVideoUpstream,
    req: &VideoCreateRequest,
) -> AppResult<()> {
    let ckpt = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "v1-5-pruned-emaonly.safetensors".to_string());
    let (w, h) = parse_wxh(req.size.as_deref().or(Some("1280x720")));
    set_input(workflow, "4", "ckpt_name", json!(ckpt))?;
    set_input(workflow, "6", "text", json!(req.prompt))?;
    set_input(workflow, "5", "width", json!(w))?;
    set_input(workflow, "5", "height", json!(h))?;
    set_input(workflow, "3", "seed", json!(rand_seed()))?;
    Ok(())
}

fn inject_i2v(
    workflow: &mut Value,
    target: &ResolvedVideoUpstream,
    req: &VideoCreateRequest,
    uploaded_name: &str,
) -> AppResult<()> {
    let ckpt = resolve_model_name(target, req.model.as_deref())
        .unwrap_or_else(|| "v1-5-pruned-emaonly.safetensors".to_string());
    set_input(workflow, "4", "ckpt_name", json!(ckpt))?;
    set_input(workflow, "6", "text", json!(req.prompt))?;
    set_input(workflow, "10", "image", json!(uploaded_name))?;
    set_input(workflow, "3", "seed", json!(rand_seed()))?;
    Ok(())
}

fn set_input(workflow: &mut Value, node_id: &str, key: &str, value: Value) -> AppResult<()> {
    let Some(node) = workflow.get_mut(node_id) else {
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

async fn submit_prompt(http: &Client, base: &str, workflow: Value) -> AppResult<String> {
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
    v.get("prompt_id")
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Upstream(format!("comfyui missing prompt_id: {text}")))
}

fn extract_media_from_history(entry: &Value) -> AppResult<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    let Some(outputs) = entry.get("outputs").and_then(|o| o.as_object()) else {
        return Ok(out);
    };
    for (_node, output) in outputs {
        for key in ["videos", "gifs", "images"] {
            if let Some(items) = output.get(key).and_then(|i| i.as_array()) {
                for item in items {
                    let filename = item
                        .get("filename")
                        .and_then(|f| f.as_str())
                        .unwrap_or("")
                        .to_string();
                    if filename.is_empty() {
                        continue;
                    }
                    // Prefer real video/gif containers over still images.
                    if key == "images"
                        && !filename.to_ascii_lowercase().ends_with(".mp4")
                        && !filename.to_ascii_lowercase().ends_with(".webm")
                        && !filename.to_ascii_lowercase().ends_with(".gif")
                    {
                        continue;
                    }
                    let subfolder = item
                        .get("subfolder")
                        .and_then(|f| f.as_str())
                        .unwrap_or("")
                        .to_string();
                    let media_type = item
                        .get("type")
                        .and_then(|f| f.as_str())
                        .unwrap_or("output")
                        .to_string();
                    out.push((filename, subfolder, media_type));
                }
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
    media_type: &str,
) -> AppResult<Vec<u8>> {
    let url = format!(
        "{}?filename={}&subfolder={}&type={}",
        join_url(base, "view"),
        urlencoding_filename(filename),
        urlencoding_filename(subfolder),
        urlencoding_filename(media_type)
    );
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui view: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Upstream(format!("comfyui view HTTP {status}")));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Upstream(format!("comfyui view body: {e}")))?;
    Ok(bytes.to_vec())
}

fn urlencoding_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

fn default_t2v_workflow() -> Value {
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
            "inputs": { "batch_size": 8, "height": 720, "width": 1280 }
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
        "10": {
            "class_type": "CreateVideo",
            "inputs": { "images": ["8", 0], "fps": 8 }
        },
        "11": {
            "class_type": "SaveVideo",
            "inputs": {
                "video": ["10", 0],
                "filename_prefix": "token_router_t2v",
                "format": "auto",
                "codec": "auto"
            }
        }
    })
}

fn default_i2v_workflow() -> Value {
    json!({
        "3": {
            "class_type": "KSampler",
            "inputs": {
                "cfg": 7.0,
                "denoise": 0.7,
                "latent_image": ["12", 0],
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
        "10": {
            "class_type": "LoadImage",
            "inputs": { "image": "input.png" }
        },
        "12": {
            "class_type": "VAEEncode",
            "inputs": { "pixels": ["10", 0], "vae": ["4", 2] }
        },
        "13": {
            "class_type": "CreateVideo",
            "inputs": { "images": ["8", 0], "fps": 8 }
        },
        "14": {
            "class_type": "SaveVideo",
            "inputs": {
                "video": ["13", 0],
                "filename_prefix": "token_router_i2v",
                "format": "auto",
                "codec": "auto"
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::config::ResolvedVideoUpstream;

    #[test]
    fn inject_t2v_sets_prompt() {
        let mut wf = default_t2v_workflow();
        let target = ResolvedVideoUpstream {
            provider: "comfyui".into(),
            base_url: Some("http://127.0.0.1:8188".into()),
            api_key: None,
            model: Some("model.safetensors".into()),
            upstream_model: None,
            workflow_file: None,
            workflow_file_i2v: None,
        };
        let req = VideoCreateRequest {
            prompt: "a cat runs".into(),
            model: None,
            seconds: Some("4".into()),
            size: Some("1280x720".into()),
            input_reference: None,
            resolution: None,
            last_frame: None,
            reference_images: vec![],
            watermark: None,
        };
        inject_t2v(&mut wf, &target, &req).unwrap();
        assert_eq!(wf["6"]["inputs"]["text"], json!("a cat runs"));
        assert_eq!(wf["4"]["inputs"]["ckpt_name"], json!("model.safetensors"));
    }
}
