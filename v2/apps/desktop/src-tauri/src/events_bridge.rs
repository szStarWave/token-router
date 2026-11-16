//! SSE 事件桥（技术方案 §6.3）：Rust 侧连 `/v2/admin/events`，按 event
//! 转成 `admin://event` Tauri event；WebView 不直连（token 不出 Rust 侧）。
//!
//! 断线重连带 `Last-Event-ID`；窗口关闭时由 tauri drop 清理。

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::AppState;

/// 订阅状态：活动任务句柄（重复订阅时先取消旧的）。
pub struct EventBridge {
    generation: Arc<Mutex<u64>>,
}

impl Default for EventBridge {
    fn default() -> Self {
        Self {
            generation: Arc::new(Mutex::new(0)),
        }
    }
}

#[tauri::command]
pub async fn admin_events_subscribe(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    bridge: tauri::State<'_, EventBridge>,
) -> Result<(), String> {
    let base = state
        .base_url
        .lock()
        .await
        .clone()
        .ok_or("service not running")?;
    let token = state.admin_token.lock().await.clone().unwrap_or_default();

    // 新一代订阅：旧的循环自动退出。
    let generation = {
        let mut guard = bridge.generation.lock().await;
        *guard += 1;
        *guard
    };
    let generation = Arc::new(tokio::sync::Mutex::new(generation));
    let generations = bridge.generation.clone();
    let app = app.clone();

    tokio::spawn(async move {
        let mut last_event_id: i64 = 0;
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(0)) // SSE 不设整体超时
            .build()
        {
            Ok(client) => client,
            Err(_) => return,
        };
        loop {
            // 新一代已启动 → 本代退出。
            {
                let current = generations.lock().await;
                if *current != *generation.lock().await {
                    return;
                }
            }
            let url = format!("{base}/v2/admin/events");
            let request = client
                .get(&url)
                .bearer_auth(&token)
                .header("Last-Event-ID", last_event_id.to_string());
            let response = match request.send().await {
                Ok(response) if response.status().is_success() => response,
                _ => {
                    // 断线退避 2s 重连。
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            // 逐块读 SSE。
            let mut stream = response;
            let mut buffer = Vec::new();
            while let Ok(Some(chunk)) = stream.chunk().await {
                {
                    let current = generations.lock().await;
                    if *current != *generation.lock().await {
                        return;
                    }
                }
                buffer.extend_from_slice(&chunk);
                // 解析完整帧（\n\n 分隔）。
                while let Some(position) = find_frame_end(&buffer) {
                    let frame = String::from_utf8_lossy(&buffer[..position]).to_string();
                    buffer.drain(..position + 2);
                    if let Some(event) = parse_frame(&frame) {
                        last_event_id = event.0;
                        let _ = app.emit(
                            "admin://event",
                            serde_json::json!({"id": event.0, "type": event.1, "data": event.2}),
                        );
                    }
                }
            }
            // 流断开：重连。
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn admin_events_unsubscribe(bridge: tauri::State<'_, EventBridge>) -> Result<(), String> {
    *bridge.generation.lock().await += 1;
    Ok(())
}

/// 找到 `\n\n` 帧尾。
fn find_frame_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|pair| pair == b"\n\n")
}

/// 解析 `id:` / `event:` / `data:` 三行帧 → (id, type, data)。
fn parse_frame(frame: &str) -> Option<(i64, String, serde_json::Value)> {
    let mut id = None;
    let mut kind = None;
    let mut data = None;
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("id: ") {
            id = value.trim().parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("event: ") {
            kind = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data: ") {
            data = Some(value.trim().to_string());
        }
    }
    match (id, kind, data) {
        (Some(id), Some(kind), Some(data)) => {
            let payload = serde_json::from_str(&data).unwrap_or(serde_json::Value::Null);
            Some((id, kind, payload))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_parsing() {
        let frame = "id: 1025\nevent: config.applied\ndata: {\"revision\":\"6f2c\",\"restart_required\":true}";
        let (id, kind, data) = parse_frame(frame).expect("parsed");
        assert_eq!(id, 1025);
        assert_eq!(kind, "config.applied");
        assert_eq!(data["revision"], "6f2c");
    }

    #[test]
    fn ping_frame_is_ignored() {
        assert!(parse_frame("event: ping\ndata: {}").is_none(), "no id line");
    }

    #[test]
    fn frame_boundary_detection() {
        // "id: 1\nevent: x\ndata: {}\n\n" = 23 字节；之后是第二帧开头。
        let buffer = b"id: 1\nevent: x\ndata: {}\n\nid: 2\nev";
        assert_eq!(find_frame_end(buffer), Some(23));
        assert_eq!(find_frame_end(&buffer[24..]), None);
    }
}
