//! 每轮请求的消息记录（migration 15）。
//!
//! 与账本同键 `request_id`；`session_key` + `created_at(秒)` 供会话维度读取。
//! 只存展平文本，不存媒体二进制；写入由 pipeline 在账本提交后尽力完成，
//! 失败只记日志，不影响请求结果。

use rusqlite::params;
use tr_contracts::admin::{RequestTranscript, TranscriptMessage};

use crate::store::{Db, StoreError};

/// 写入（或覆盖）一轮消息记录。
pub fn insert(db: &Db, transcript: &RequestTranscript) -> Result<(), StoreError> {
    let input = serde_json::to_string(&transcript.input)?;
    let output = serde_json::to_string(&transcript.output)?;
    let connection = db.write();
    connection.execute(
        "INSERT OR REPLACE INTO request_transcripts \
         (request_id, session_key, created_at, input_json, output_json, truncated) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            transcript.request_id,
            transcript.session_key,
            transcript.created_at / 1000,
            input,
            output,
            i64::from(transcript.truncated),
        ],
    )?;
    Ok(())
}

/// 读一轮消息记录；没有记录返回 `None`。
pub fn get(db: &Db, request_id: &str) -> Result<Option<RequestTranscript>, StoreError> {
    let connection = db.read();
    let mut statement = connection.prepare(
        "SELECT session_key, created_at, input_json, output_json, truncated \
           FROM request_transcripts WHERE request_id = ?1",
    )?;
    let mut rows = statement.query(params![request_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let session_key: String = row.get(0)?;
    let created_at: i64 = row.get(1)?;
    let input_json: String = row.get(2)?;
    let output_json: String = row.get(3)?;
    let truncated: i64 = row.get(4)?;
    Ok(Some(RequestTranscript {
        request_id: request_id.to_string(),
        session_key,
        created_at: created_at * 1000,
        input: serde_json::from_str(&input_json)?,
        output: serde_json::from_str(&output_json)?,
        truncated: truncated != 0,
        recorded: true,
    }))
}

/// 没记录时的空结果（关闭记录、或早于该能力的请求）。
pub fn empty(request_id: &str) -> RequestTranscript {
    RequestTranscript {
        request_id: request_id.to_string(),
        session_key: String::new(),
        created_at: 0,
        input: Vec::new(),
        output: Vec::new(),
        truncated: false,
        recorded: false,
    }
}

/// 供测试/工具构造一条纯文本消息。
pub fn text_message(role: &str, text: impl Into<String>) -> TranscriptMessage {
    TranscriptMessage {
        role: role.to_string(),
        name: None,
        text: text.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(request_id: &str) -> RequestTranscript {
        RequestTranscript {
            request_id: request_id.to_string(),
            session_key: "sess-1".to_string(),
            created_at: 1_700_000_000_000,
            input: vec![
                text_message("system", "be brief"),
                text_message("user", "hello"),
            ],
            output: vec![text_message("assistant", "hi")],
            truncated: false,
            recorded: true,
        }
    }

    #[test]
    fn round_trips_messages_and_seconds() {
        let db = Db::open_in_memory().expect("db");
        insert(&db, &sample("req-1")).expect("insert");
        let back = get(&db, "req-1").expect("get").expect("recorded");
        assert_eq!(back.request_id, "req-1");
        assert_eq!(back.session_key, "sess-1");
        // 秒级存储，读回补齐毫秒。
        assert_eq!(back.created_at, 1_700_000_000_000);
        assert_eq!(back.input.len(), 2);
        assert_eq!(back.input[1].text, "hello");
        assert_eq!(back.output[0].text, "hi");
        assert!(back.recorded);
    }

    #[test]
    fn missing_request_is_not_recorded() {
        let db = Db::open_in_memory().expect("db");
        assert!(get(&db, "nope").expect("get").is_none());
        assert!(!empty("nope").recorded);
    }

    #[test]
    fn insert_replaces_same_request() {
        let db = Db::open_in_memory().expect("db");
        insert(&db, &sample("req-1")).expect("insert");
        let mut updated = sample("req-1");
        updated.output = vec![text_message("assistant", "updated")];
        updated.truncated = true;
        insert(&db, &updated).expect("replace");
        let back = get(&db, "req-1").expect("get").expect("recorded");
        assert_eq!(back.output[0].text, "updated");
        assert!(back.truncated);
    }
}
