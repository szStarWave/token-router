//! Video passthrough. Callers speak the upstream protocol; this module only forwards.

pub(crate) mod forward;
pub mod tier;

use std::sync::Arc;

use crate::gateway::config::ResolvedVideoUpstream;
use crate::gateway::edge_load::EdgeInferenceTracker;
use crate::gateway::error::AppResult;

use self::tier::VideoTier;

pub use forward::{
    VideoForwardRequest, VideoForwardResponse, parse_forward_method, resolve_forward_url,
};
pub use tier::{resolve_video_tier, VideoTier as ResolvedVideoTier};

#[derive(Clone)]
pub struct VideoClient {
    http: reqwest::Client,
    edge_load: Arc<EdgeInferenceTracker>,
}

impl VideoClient {
    pub fn new(edge_load: Arc<EdgeInferenceTracker>) -> Self {
        Self {
            http: forward::forward_client(),
            edge_load,
        }
    }

    pub async fn forward(
        &self,
        target: &ResolvedVideoUpstream,
        tier: VideoTier,
        req: &VideoForwardRequest,
    ) -> AppResult<VideoForwardResponse> {
        let _guard = if tier == VideoTier::Edge {
            Some(self.edge_load.begin())
        } else {
            None
        };
        forward::forward_video(&self.http, target, req).await
    }
}
