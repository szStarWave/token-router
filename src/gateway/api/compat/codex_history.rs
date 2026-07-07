use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::RwLock;

use serde_json::Value;

use crate::gateway::api::compat::response_id_from_chat_id;

const MAX_CACHED_RESPONSES: usize = 512;

#[derive(Debug, Clone, Default)]
struct CachedResponse {
    calls_by_id: HashMap<String, Value>,
    call_order: Vec<String>,
}

#[derive(Debug, Default)]
struct CodexChatHistoryInner {
    responses: HashMap<String, CachedResponse>,
    response_order: VecDeque<String>,
    call_index: HashMap<String, String>,
}

/// Restores missing Responses `function_call` items before `function_call_output`
/// when Codex sends follow-ups with `previous_response_id` only.
#[derive(Debug, Default)]
pub struct CodexChatHistoryStore {
    inner: RwLock<CodexChatHistoryInner>,
}

impl CodexChatHistoryStore {
    pub fn record_calls_from_input(&self, body: &Value) {
        let Some(response_id) = body
            .get("previous_response_id")
            .or_else(|| body.get("id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            self.record_input_calls(body, None);
            return;
        };
        self.record_input_calls(body, Some(response_id));
    }

    pub fn record_response(&self, response: &Value) {
        let raw_id = response.get("id").and_then(|v| v.as_str());
        let response_id = response_id_from_chat_id(raw_id);
        if response_id == "resp_unknown" {
            return;
        }
        let calls = response
            .get("output")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(cached_call_item)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let call_count = calls.len();
        if calls.is_empty() {
            return;
        }
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.insert_calls(&response_id, calls);
        tracing::debug!(
            response_id = %response_id,
            calls = call_count,
            "codex_history record response"
        );
    }

    /// Incrementally record a single call item during SSE streaming.
    pub fn record_call_item(&self, response_id: &str, item: &Value) {
        let response_id = response_id_from_chat_id(Some(response_id));
        let Some(call) = cached_call_item(item) else {
            return;
        };
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let added = inner.insert_calls(&response_id, vec![call]);
        if added > 0 {
            tracing::debug!(
                response_id = %response_id,
                call_id = %item.get("call_id").and_then(|v| v.as_str()).unwrap_or("?"),
                "codex_history record call item"
            );
        }
    }

    pub fn enrich_request(&self, body: &mut Value) {
        let previous_response_id = body
            .get("previous_response_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let Some(input) = body.get_mut("input") else {
            return;
        };
        let original = std::mem::take(input);
        let items = match original {
            Value::Array(items) => items,
            Value::Object(object) => vec![Value::Object(object)],
            other => {
                *input = other;
                return;
            }
        };

        let output_call_ids: HashSet<String> = items
            .iter()
            .filter(|item| {
                item.get("type").and_then(|v| v.as_str()) == Some("function_call_output")
                    || item.get("role").and_then(|v| v.as_str()) == Some("tool")
            })
            .filter_map(item_call_id)
            .collect();
        let existing_call_ids: HashSet<String> = items
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
            .filter_map(item_call_id)
            .collect();

        let lookup = self.lookup(previous_response_id.as_deref(), &output_call_ids);
        let restore_group = lookup.restore_group(&output_call_ids, &existing_call_ids);
        let restored_count = restore_group.len();

        if previous_response_id.is_some() && !output_call_ids.is_empty() && restored_count == 0 {
            let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
            let cached_count = previous_response_id
                .as_ref()
                .and_then(|id| inner.responses.get(id))
                .map(|c| c.calls_by_id.len())
                .unwrap_or(0);
            let output_call_ids_list: Vec<&str> = output_call_ids.iter().map(|s| s.as_str()).collect();
            tracing::warn!(
                previous = %previous_response_id.as_deref().unwrap_or("?"),
                output_call_ids = ?output_call_ids_list,
                cached_responses = cached_count,
                "codex_history enrich miss"
            );
        }

        let mut new_items = Vec::with_capacity(items.len() + restore_group.len());
        let mut inserted = HashSet::new();

        for (call_id, call_item) in restore_group {
            if existing_call_ids.contains(&call_id) || !inserted.insert(call_id.clone()) {
                continue;
            }
            new_items.push(call_item);
        }

        for item in items {
            if let Some(call_id) = item_call_id(&item) {
                if item.get("type").and_then(|v| v.as_str()) == Some("function_call_output")
                    && !existing_call_ids.contains(&call_id)
                    && !inserted.contains(&call_id)
                {
                    if let Some(call) = lookup.call_by_id(&call_id) {
                        new_items.push(call);
                        inserted.insert(call_id);
                    }
                }
            }
            new_items.push(item);
        }

        *input = Value::Array(new_items);

        if restored_count > 0 {
            tracing::debug!(
                previous = %previous_response_id.as_deref().unwrap_or("?"),
                restored = restored_count,
                "codex_history enrich hit"
            );
        }
    }
}

impl CodexChatHistoryInner {
    fn insert_calls(&mut self, response_id: &str, calls: Vec<(String, Value)>) -> usize {
        if calls.is_empty() {
            return 0;
        }
        let entry = self.responses.entry(response_id.to_string()).or_default();
        let mut added = 0usize;
        for (call_id, item) in calls {
            if entry.calls_by_id.contains_key(&call_id) {
                continue;
            }
            entry.call_order.push(call_id.clone());
            entry.calls_by_id.insert(call_id.clone(), item);
            self.call_index.insert(call_id, response_id.to_string());
            added += 1;
        }
        if added > 0 {
            self.response_order.retain(|id| id != response_id);
            self.response_order.push_back(response_id.to_string());
            while self.response_order.len() > MAX_CACHED_RESPONSES {
                if let Some(old) = self.response_order.pop_front() {
                    if let Some(removed) = self.responses.remove(&old) {
                        for call_id in removed.call_order {
                            self.call_index.remove(&call_id);
                        }
                    }
                }
            }
        }
        added
    }
}

impl CodexChatHistoryStore {
    fn record_input_calls(&self, body: &Value, response_id: Option<&str>) {
        let Some(items) = body.get("input").and_then(|v| v.as_array()) else {
            return;
        };
        let calls: Vec<_> = items.iter().filter_map(cached_call_item).collect();
        if calls.is_empty() {
            return;
        }
        let id = response_id
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                body.get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            });
        let response_id = id.unwrap_or_else(|| "orphan".to_string());
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.insert_calls(&response_id, calls);
    }

    fn lookup(&self, previous_response_id: Option<&str>, requested: &HashSet<String>) -> CachedLookup {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let previous = previous_response_id.and_then(|id| inner.responses.get(id).cloned());
        let mut fallback = CachedResponse::default();
        for call_id in requested {
            if let Some(response_id) = inner.call_index.get(call_id) {
                if let Some(cached) = inner.responses.get(response_id) {
                    if let Some(item) = cached.calls_by_id.get(call_id) {
                        fallback.calls_by_id.insert(call_id.clone(), item.clone());
                        fallback.call_order.push(call_id.clone());
                    }
                }
            }
        }
        CachedLookup { previous, fallback }
    }
}

#[derive(Debug, Clone, Default)]
struct CachedLookup {
    previous: Option<CachedResponse>,
    fallback: CachedResponse,
}

impl CachedLookup {
    fn restore_group(
        &self,
        output_call_ids: &HashSet<String>,
        existing_call_ids: &HashSet<String>,
    ) -> Vec<(String, Value)> {
        let source = self
            .previous
            .as_ref()
            .filter(|c| !c.calls_by_id.is_empty())
            .or_else(|| {
                if self.fallback.calls_by_id.is_empty() {
                    None
                } else {
                    Some(&self.fallback)
                }
            });
        let Some(source) = source else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for call_id in &source.call_order {
            if !output_call_ids.contains(call_id) || existing_call_ids.contains(call_id) {
                continue;
            }
            if let Some(item) = source.calls_by_id.get(call_id) {
                out.push((call_id.clone(), item.clone()));
            }
        }
        out
    }

    fn call_by_id(&self, call_id: &str) -> Option<Value> {
        self.previous
            .as_ref()
            .and_then(|c| c.calls_by_id.get(call_id).cloned())
            .or_else(|| self.fallback.calls_by_id.get(call_id).cloned())
    }
}

fn cached_call_item(item: &Value) -> Option<(String, Value)> {
    if item.get("type").and_then(|v| v.as_str()) != Some("function_call") {
        return None;
    }
    let call_id = item_call_id(item)?;
    Some((call_id, item.clone()))
}

fn item_call_id(item: &Value) -> Option<String> {
    item.get("call_id")
        .or_else(|| item.get("tool_call_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn restores_missing_function_call_before_output() {
        let store = CodexChatHistoryStore::default();
        store.record_response(&json!({
            "id": "resp_1",
            "output": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "read",
                "arguments": "{}"
            }]
        }));

        let mut body = json!({
            "previous_response_id": "resp_1",
            "input": [{
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "ok"
            }]
        });
        store.enrich_request(&mut body);
        let items = body["input"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "function_call");
        assert_eq!(items[1]["type"], "function_call_output");
    }

    #[test]
    fn record_call_item_stores_single_call() {
        let store = CodexChatHistoryStore::default();
        store.record_call_item("resp_chatcmpl-abc", &json!({
            "type": "function_call",
            "call_id": "call_1",
            "name": "exec",
            "arguments": "{}"
        }));
        let mut body = json!({
            "previous_response_id": "resp_chatcmpl-abc",
            "input": [{
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "ok"
            }]
        });
        store.enrich_request(&mut body);
        let items = body["input"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "function_call");
    }

    #[test]
    fn record_call_item_auto_prefixes_response_id() {
        let store = CodexChatHistoryStore::default();
        store.record_call_item("chatcmpl-xyz", &json!({
            "type": "function_call",
            "call_id": "call_2",
            "name": "read",
            "arguments": "{}"
        }));
        let mut body = json!({
            "previous_response_id": "resp_chatcmpl-xyz",
            "input": [{
                "type": "function_call_output",
                "call_id": "call_2",
                "output": "ok"
            }]
        });
        store.enrich_request(&mut body);
        let items = body["input"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "function_call");
    }

    #[test]
    fn enrich_two_turn_tool_session() {
        let store = CodexChatHistoryStore::default();
        // Turn 1: record response with two parallel tool calls
        store.record_response(&json!({
            "id": "resp_chatcmpl-turn1",
            "output": [
                {"type": "function_call", "call_id": "call_a", "name": "search", "arguments": "{}"},
                {"type": "function_call", "call_id": "call_b", "name": "read", "arguments": "{}"}
            ]
        }));
        // Turn 2: only function_call_outputs, no function_call items
        let mut body = json!({
            "previous_response_id": "resp_chatcmpl-turn1",
            "input": [
                {"type": "function_call_output", "call_id": "call_a", "output": "result_a"},
                {"type": "function_call_output", "call_id": "call_b", "output": "result_b"}
            ]
        });
        store.enrich_request(&mut body);
        let items = body["input"].as_array().unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0]["type"], "function_call");
        assert_eq!(items[0]["call_id"], "call_a");
        assert_eq!(items[1]["type"], "function_call");
        assert_eq!(items[1]["call_id"], "call_b");
        assert_eq!(items[2]["type"], "function_call_output");
        assert_eq!(items[2]["call_id"], "call_a");
        assert_eq!(items[3]["type"], "function_call_output");
        assert_eq!(items[3]["call_id"], "call_b");
    }

    #[test]
    fn response_id_from_chat_id_normalizes() {
        assert_eq!(response_id_from_chat_id(Some("chatcmpl-abc")), "resp_chatcmpl-abc");
        assert_eq!(response_id_from_chat_id(Some("resp_chatcmpl-abc")), "resp_chatcmpl-abc");
        assert_eq!(response_id_from_chat_id(Some("")), "resp_unknown");
        assert_eq!(response_id_from_chat_id(None), "resp_unknown");
    }
}
