use serde::{Deserialize, Serialize};

/// Uploaded or URL reference image for image-to-video.
#[derive(Debug, Clone)]
pub enum ImageRef {
    Bytes {
        bytes: Vec<u8>,
        mime: String,
        filename: String,
    },
    Url(String),
}

#[derive(Debug, Clone)]
pub struct VideoCreateRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub seconds: Option<String>,
    pub size: Option<String>,
    /// OpenAI-style first-frame reference (`image_url` / `input_reference`).
    pub input_reference: Option<ImageRef>,
    /// MiniMax-H3 explicit resolution (`768P` / `2K`). Prefer over deriving from `size`.
    pub resolution: Option<String>,
    /// MiniMax-H3 last frame URL / data URI.
    pub last_frame: Option<ImageRef>,
    /// MiniMax-H3 reference images (max 9 upstream). Mutually exclusive with first/last frame.
    pub reference_images: Vec<ImageRef>,
    /// When `Some(true)`, mapped to vendor watermark fields (MiniMax `aigc_watermark`, DashScope/Seedance `watermark`).
    pub watermark: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoErrorObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoObject {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub progress: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VideoErrorObject>,
}

impl VideoObject {
    pub fn from_job(job: &VideoJob) -> Self {
        Self {
            id: job.id.clone(),
            object: "video".into(),
            created_at: job.created_at,
            status: job.status.clone(),
            model: Some(job.model.clone()).filter(|m| !m.is_empty()),
            progress: job.progress,
            seconds: job.seconds.clone(),
            size: job.size.clone(),
            error: job.error.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoListResponse {
    pub object: String,
    pub data: Vec<VideoObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_id: Option<String>,
    pub has_more: bool,
}

/// Persisted job record (gateway-local).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoJob {
    pub id: String,
    pub provider: String,
    pub tier: String,
    #[serde(default)]
    pub upstream_task_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub progress: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub seconds: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub error: Option<VideoErrorObject>,
    #[serde(default)]
    pub result_url: Option<String>,
    #[serde(default)]
    pub local_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl VideoJob {
    pub fn to_object(&self) -> VideoObject {
        VideoObject::from_job(self)
    }

    pub fn is_terminal(&self) -> bool {
        is_terminal_status(&self.status)
    }

    pub fn touch(&mut self) {
        self.updated_at = now_unix();
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "canceled"
    )
}

pub fn new_video_id() -> String {
    format!("video_{}", uuid::Uuid::new_v4().simple())
}
