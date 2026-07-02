use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

const WEIXIN_FEEDBACK_WEBHOOK: &str =
    "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=e39bf4ef-785e-4488-ab46-4360ac702fb4";
const MAX_FEEDBACK_CHARS: usize = 3000;
const MAX_MARKDOWN_CHARS: usize = 3800;
const MAX_LOG_LINES: usize = 3000;
const MAX_UPLOAD_BYTES: usize = 20 << 20;

#[derive(Serialize)]
struct WeixinWebhookBody {
    msgtype: &'static str,
    markdown: MarkdownContent,
}

#[derive(Serialize)]
struct MarkdownContent {
    content: String,
}

#[derive(Deserialize)]
struct WeixinWebhookResponse {
    errcode: i32,
    errmsg: String,
}

#[derive(Deserialize)]
struct WeixinMediaUploadResponse {
    errcode: i32,
    errmsg: String,
    media_id: Option<String>,
}

fn category_label(category: &str) -> &str {
    match category {
        "download_failed" => "模型下载失败",
        "start_failed" => "模型启动失败",
        "usage_issue" => "模型不能正常使用",
        _ => "其它",
    }
}

fn sanitize_feedback_text(s: &str) -> String {
    s.replace('<', "＜").replace('>', "＞")
}

fn system_info() -> (String, f64) {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let _gpu = "N/A".to_string();
    let ram_gb = total_ram_gb().unwrap_or(0.0);
    (format!("{os} / {arch}"), ram_gb)
}

#[cfg(windows)]
fn total_ram_gb() -> Option<f64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    unsafe {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut status).is_ok() {
            return Some(status.ullTotalPhys as f64 / (1024.0 * 1024.0 * 1024.0));
        }
    }
    None
}

#[cfg(not(windows))]
fn total_ram_gb() -> Option<f64> {
    None
}

fn build_feedback_markdown(content: &str, category: &str) -> String {
    let (hw, ram_gb) = system_info();
    let version = env!("CARGO_PKG_VERSION");
    let mut lines = vec![
        "Token Router 用户反馈".to_string(),
        ">类型:<font color=\"comment\">用户反馈</font>".to_string(),
        format!(
            ">分类:<font color=\"comment\">{}</font>",
            sanitize_feedback_text(category_label(category))
        ),
        format!(
            ">版本:<font color=\"comment\">{}</font>",
            sanitize_feedback_text(version)
        ),
        ">硬件信息:".to_string(),
        format!(
            ">  系统: <font color=\"comment\">{}</font>",
            sanitize_feedback_text(&hw)
        ),
        format!(">  内存: <font color=\"comment\">{ram_gb:.1}GB</font>"),
        ">内容:".to_string(),
    ];
    for line in content.lines() {
        lines.push(format!(">{}", sanitize_feedback_text(line)));
    }
    let mut markdown = lines.join("\n");
    if markdown.chars().count() > MAX_MARKDOWN_CHARS {
        markdown = markdown
            .chars()
            .take(MAX_MARKDOWN_CHARS)
            .collect::<String>()
            + "\n>…";
    }
    markdown
}

fn gateway_log_path() -> Result<PathBuf, String> {
    let config = token_router::gateway::AppConfig::load().map_err(|e| e.to_string())?;
    Ok(config.data_dir.join("logs").join("gateway.log"))
}

fn read_last_lines(path: &std::path::Path, max_lines: usize) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if lines.len() >= max_lines {
            lines.remove(0);
        }
        lines.push(line);
    }
    Ok(lines)
}

fn collect_logs() -> Result<(Vec<u8>, String), String> {
    let timestamp = chrono_lite_timestamp();
    let filename = format!("token-router-feedback-logs-{timestamp}.txt");
    let mut buf = Vec::new();

    if let Ok(path) = gateway_log_path() {
        if path.is_file() {
            buf.extend_from_slice("===== Gateway Log =====\n".as_bytes());
            match read_last_lines(&path, MAX_LOG_LINES) {
                Ok(lines) => {
                    for line in lines {
                        buf.extend_from_slice(line.as_bytes());
                        buf.push(b'\n');
                    }
                }
                Err(e) => {
                    buf.extend_from_slice(format!("读取日志失败: {e}\n").as_bytes());
                }
            }
            buf.push(b'\n');
        }
    }

    if buf.is_empty() {
        buf.extend_from_slice(b"(no logs available)\n");
    }

    if buf.len() > MAX_UPLOAD_BYTES {
        let truncated = MAX_UPLOAD_BYTES.saturating_sub(100);
        buf.truncate(truncated);
        buf.extend_from_slice(b"\n... (log truncated due to size limit)\n");
    }

    Ok((buf, filename))
}

fn chrono_lite_timestamp() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", dur.as_secs())
}

fn extract_webhook_key(url: &str) -> Option<&str> {
    url.split("key=").nth(1).map(|k| k.split('&').next().unwrap_or(k))
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn post_markdown(client: &Client, markdown: &str) -> Result<(), String> {
    let payload = WeixinWebhookBody {
        msgtype: "markdown",
        markdown: MarkdownContent {
            content: markdown.to_string(),
        },
    };
    let resp = client
        .post(WEIXIN_FEEDBACK_WEBHOOK)
        .json(&payload)
        .send()
        .map_err(|e| format!("send feedback: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("webhook HTTP {}", resp.status()));
    }
    let wr: WeixinWebhookResponse = resp.json().map_err(|e| e.to_string())?;
    if wr.errcode != 0 {
        return Err(format!("weixin API: {} ({})", wr.errmsg, wr.errcode));
    }
    Ok(())
}

fn upload_media_file(client: &Client, content: &[u8], filename: &str) -> Result<String, String> {
    let key = extract_webhook_key(WEIXIN_FEEDBACK_WEBHOOK).ok_or("invalid webhook URL")?;
    let upload_url = format!(
        "https://qyapi.weixin.qq.com/cgi-bin/webhook/upload_media?key={key}&type=file"
    );

    let part = reqwest::blocking::multipart::Part::bytes(content.to_vec())
        .file_name(filename.to_string())
        .mime_str("text/plain")
        .map_err(|e| e.to_string())?;
    let form = reqwest::blocking::multipart::Form::new().part("media", part);

    let resp = client
        .post(&upload_url)
        .multipart(form)
        .send()
        .map_err(|e| format!("upload request: {e}"))?;

    let upload_resp: WeixinMediaUploadResponse =
        resp.json().map_err(|e| format!("decode upload response: {e}"))?;
    if upload_resp.errcode != 0 {
        return Err(format!(
            "upload failed: {} ({})",
            upload_resp.errmsg, upload_resp.errcode
        ));
    }
    upload_resp
        .media_id
        .ok_or_else(|| "missing media_id".to_string())
}

fn send_file_message(client: &Client, media_id: &str) -> Result<(), String> {
    let payload = serde_json::json!({
        "msgtype": "file",
        "file": { "media_id": media_id }
    });
    let resp = client
        .post(WEIXIN_FEEDBACK_WEBHOOK)
        .json(&payload)
        .send()
        .map_err(|e| e.to_string())?;
    let wr: WeixinWebhookResponse = resp.json().map_err(|e| e.to_string())?;
    if wr.errcode != 0 {
        return Err(format!("send file message failed: {} ({})", wr.errmsg, wr.errcode));
    }
    Ok(())
}

#[tauri::command]
pub fn feedback_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub fn feedback_submit(content: String, category: Option<String>) -> Result<(), String> {
    let body = content.trim();
    if body.is_empty() {
        return Err("feedback content is empty".into());
    }
    if body.chars().count() > MAX_FEEDBACK_CHARS {
        return Err(format!(
            "feedback is too long (max {MAX_FEEDBACK_CHARS} characters)"
        ));
    }

    let cat = category.as_deref().unwrap_or("other");
    let markdown = build_feedback_markdown(body, cat);
    let client = http_client();

    post_markdown(&client, &markdown)?;

    if let Ok((log_content, log_filename)) = collect_logs() {
        if let Ok(media_id) = upload_media_file(&client, &log_content, &log_filename) {
            let _ = send_file_message(&client, &media_id);
        }
    }

    Ok(())
}
