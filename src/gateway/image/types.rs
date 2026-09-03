use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerateRequest {
    #[serde(default)]
    pub model: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub n: Option<u32>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub response_format: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImageBytes {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub filename: String,
}

#[derive(Debug, Clone)]
pub struct ImageEditRequest {
    pub model: Option<String>,
    pub prompt: String,
    pub image: ImageBytes,
    pub mask: Option<ImageBytes>,
    pub n: Option<u32>,
    pub size: Option<String>,
    pub response_format: Option<String>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagesResponse {
    pub created: i64,
    pub data: Vec<ImageData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}

impl ImagesResponse {
    pub fn now_with_data(data: Vec<ImageData>) -> Self {
        Self {
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            data,
        }
    }
}

pub fn wants_b64(response_format: Option<&str>) -> bool {
    matches!(
        response_format.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("b64_json") | Some("base64")
    )
}
