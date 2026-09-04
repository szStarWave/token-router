use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::gateway::error::{AppError, AppResult};
use crate::gateway::video::types::VideoJob;

pub struct VideoJobStore {
    dir: PathBuf,
    files_dir: PathBuf,
    lock: Mutex<()>,
}

impl VideoJobStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<std::sync::Arc<Self>> {
        let dir = data_dir.join("videos");
        let files_dir = dir.join("files");
        fs::create_dir_all(&dir)?;
        fs::create_dir_all(&files_dir)?;
        Ok(std::sync::Arc::new(Self {
            dir,
            files_dir,
            lock: Mutex::new(()),
        }))
    }

    #[cfg(test)]
    pub fn open_temp(dir: PathBuf) -> std::sync::Arc<Self> {
        let files_dir = dir.join("files");
        let _ = fs::create_dir_all(&dir);
        let _ = fs::create_dir_all(&files_dir);
        std::sync::Arc::new(Self {
            dir,
            files_dir,
            lock: Mutex::new(()),
        })
    }

    pub fn files_dir(&self) -> &Path {
        &self.files_dir
    }

    pub fn local_file_path(&self, video_id: &str) -> PathBuf {
        self.files_dir.join(format!("{video_id}.mp4"))
    }

    fn job_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn save(&self, job: &VideoJob) -> AppResult<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| AppError::Internal(anyhow::anyhow!("video store lock poisoned")))?;
        let path = self.job_path(&job.id);
        let raw = serde_json::to_vec_pretty(job)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize video job: {e}")))?;
        fs::write(&path, raw)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("write video job: {e}")))?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> AppResult<Option<VideoJob>> {
        let path = self.job_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read video job: {e}")))?;
        let job: VideoJob = serde_json::from_str(&raw)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("parse video job: {e}")))?;
        Ok(Some(job))
    }

    pub fn require(&self, id: &str) -> AppResult<VideoJob> {
        self.get(id)?
            .ok_or_else(|| AppError::NotFound(format!("video `{id}` not found")))
    }

    /// Remove job JSON and any cached mp4 for `id`.
    pub fn delete(&self, id: &str) -> AppResult<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| AppError::Internal(anyhow::anyhow!("video store lock poisoned")))?;
        let path = self.job_path(id);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("delete video job: {e}")))?;
        }
        let media = self.local_file_path(id);
        if media.exists() {
            let _ = fs::remove_file(&media);
        }
        Ok(())
    }

    /// List jobs sorted by `created_at` then `id`.
    pub fn list(
        &self,
        limit: usize,
        after: Option<&str>,
        order_desc: bool,
    ) -> AppResult<(Vec<VideoJob>, bool)> {
        let mut jobs = self.scan_all()?;
        jobs.sort_by(|a, b| {
            let cmp = a
                .created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id));
            if order_desc {
                cmp.reverse()
            } else {
                cmp
            }
        });

        let start = if let Some(after) = after {
            jobs.iter()
                .position(|j| j.id == after)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let end = (start + limit).min(jobs.len());
        let has_more = end < jobs.len();
        Ok((jobs[start..end].to_vec(), has_more))
    }

    fn scan_all(&self) -> AppResult<Vec<VideoJob>> {
        let mut out = Vec::new();
        let entries = fs::read_dir(&self.dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("scan video store: {e}"))
        })?;
        for entry in entries {
            let entry = entry
                .map_err(|e| AppError::Internal(anyhow::anyhow!("scan video entry: {e}")))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = match fs::read_to_string(&path) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if let Ok(job) = serde_json::from_str::<VideoJob>(&raw) {
                out.push(job);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::video::types::VideoJob;

    fn sample(id: &str, created_at: i64) -> VideoJob {
        VideoJob {
            id: id.into(),
            provider: "openai".into(),
            tier: "cloud".into(),
            upstream_task_id: None,
            status: "queued".into(),
            progress: 0,
            model: "sora-2".into(),
            seconds: Some("8".into()),
            size: Some("1280x720".into()),
            prompt: Some("hi".into()),
            error: None,
            result_url: None,
            local_path: None,
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn list_pagination_after_and_order() {
        let dir = std::env::temp_dir().join(format!(
            "token-router-video-store-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = VideoJobStore::open_temp(dir.clone());
        store.save(&sample("video_a", 100)).unwrap();
        store.save(&sample("video_b", 200)).unwrap();
        store.save(&sample("video_c", 300)).unwrap();

        let (desc, more) = store.list(2, None, true).unwrap();
        assert_eq!(desc.len(), 2);
        assert_eq!(desc[0].id, "video_c");
        assert_eq!(desc[1].id, "video_b");
        assert!(more);

        let (next, more2) = store.list(2, Some("video_b"), true).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].id, "video_a");
        assert!(!more2);

        let (asc, _) = store.list(10, None, false).unwrap();
        assert_eq!(asc[0].id, "video_a");
        assert_eq!(asc[2].id, "video_c");

        store.delete("video_b").unwrap();
        assert!(store.get("video_b").unwrap().is_none());
        let (after_del, _) = store.list(10, None, true).unwrap();
        assert_eq!(after_del.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }
}
