//! 会话键推导（§8.1）。
//!
//! 优先使用服务端下发、客户端带回的 `server_session_id`；否则用内容指纹推导。
//! 推导不出有效内容时返回 `Fallback` + `collision_suspect = true`，由调用方
//! 每个键只 warn 一次（禁止静默失效）。

use sha2::{Digest, Sha256};
use tr_contracts::{
    CanonicalRequest, CanonicalRole, ContentBlock, SessionKey, SessionKeyInput, SessionKeySource,
};

/// 推导结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedSession {
    pub key: SessionKey,
    pub source: SessionKeySource,
    /// 键的碰撞风险（内容过少，多个会话可能落到同一键）。
    pub collision_suspect: bool,
    /// 前缀指纹（用于缓存锚点，不是会话键）。
    pub prefix_hash: String,
}

/// 从请求构造推导输入。`server_session_id` 来自响应头/请求头，由 `server` 提取。
pub fn build_input(
    request: &CanonicalRequest,
    server_session_id: Option<String>,
) -> SessionKeyInput {
    let system_text = request
        .system
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let system_text = if system_text.is_empty() {
        first_message_text(request, CanonicalRole::System).unwrap_or_default()
    } else {
        system_text
    };
    let user_text = first_message_text(request, CanonicalRole::User).unwrap_or_default();

    SessionKeyInput {
        client_id: request.meta.client_id.clone().or_else(|| {
            request
                .passthrough
                .get("user")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        }),
        workspace_hint: request.meta.workspace_hint.clone(),
        first_system_hash: hash(&system_text),
        first_user_hash: hash(&user_text),
        tool_names: request.tools.iter().map(|tool| tool.name.clone()).collect(),
        server_session_id,
    }
}

/// 推导会话键。
pub fn derive(input: &SessionKeyInput) -> DerivedSession {
    let prefix_hash = hash(&format!(
        "{}\u{1f}{}\u{1f}{}",
        input.first_system_hash,
        input.first_user_hash,
        input.tool_names.join(",")
    ));

    if let Some(server_id) = &input.server_session_id
        && !server_id.is_empty()
    {
        return DerivedSession {
            key: hash(&format!("server:{server_id}")),
            source: SessionKeySource::ServerId,
            collision_suspect: false,
            prefix_hash,
        };
    }

    let key = hash(&format!(
        "content:{}:{}:{}:{}:{}",
        input.first_system_hash,
        input.first_user_hash,
        input.tool_names.join(","),
        input.workspace_hint.clone().unwrap_or_default(),
        input.client_id.clone().unwrap_or_default()
    ));

    let empty = hash("");
    let weak = input.first_user_hash == empty && input.first_system_hash == empty;

    DerivedSession {
        key,
        source: if weak {
            SessionKeySource::Fallback
        } else {
            SessionKeySource::Content
        },
        collision_suspect: weak,
        prefix_hash,
    }
}

fn first_message_text(request: &CanonicalRequest, role: CanonicalRole) -> Option<String> {
    request
        .messages
        .iter()
        .find(|message| message.role == role)
        .map(|message| {
            message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
}

fn hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tr_contracts::{CanonicalMessage, RequestMeta};

    fn request(user_text: &str) -> CanonicalRequest {
        CanonicalRequest {
            messages: vec![CanonicalMessage {
                role: CanonicalRole::User,
                blocks: vec![ContentBlock::Text {
                    text: user_text.to_string(),
                    cache_control: None,
                }],
                ..CanonicalMessage::default()
            }],
            meta: RequestMeta::default(),
            ..CanonicalRequest::default()
        }
    }

    #[test]
    fn same_content_same_key() {
        let a = derive(&build_input(&request("hello"), None));
        let b = derive(&build_input(&request("hello"), None));
        assert_eq!(a.key, b.key);
        assert_eq!(a.source, SessionKeySource::Content);

        let c = derive(&build_input(&request("different"), None));
        assert_ne!(a.key, c.key);
    }

    #[test]
    fn server_id_wins() {
        let derived = derive(&build_input(&request("hello"), Some("srv-1".to_string())));
        assert_eq!(derived.source, SessionKeySource::ServerId);
        assert_eq!(derived.key.len(), 64);
    }

    #[test]
    fn empty_request_is_collision_suspect() {
        let empty = CanonicalRequest::default();
        let derived = derive(&build_input(&empty, None));
        assert_eq!(derived.source, SessionKeySource::Fallback);
        assert!(derived.collision_suspect);
    }
}
