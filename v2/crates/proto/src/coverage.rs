//! 字段覆盖探针（S0-4）。
//!
//! 目的不是“实现编解码”，而是回答一个可证伪的问题：
//! **三协议的真实请求/响应里，哪些字段进不了 canonical？**
//!
//! 做法：为每个协议维护一张 `wire 路径 → canonical 路径` 映射表，然后把 fixture
//! 里的每个叶子路径拿去匹配。匹配不上的就是**已知缺口**，写进 S0 gap report；
//! 匹配上的比例就是 G0 的 ≥95% 门禁。
//!
//! 映射规则：
//! - 数组路径统一写成 `key[]`（例如 `messages[].content[].text`）；
//! - `subtree` 表示整个子树原样搬运（JSON Schema、metadata 等），不逐个叶子映射；
//! - 一个 wire 路径可以映射到 `passthrough.*`——那也是“有处置”，不是丢弃。

use std::collections::BTreeSet;

use serde_json::Value;
use tr_contracts::IngressProtocol;

/// 一条 wire → canonical 映射。
#[derive(Debug, Clone, Copy)]
pub struct Mapping {
    /// wire 路径，数组用 `key[]`。
    pub wire: &'static str,
    /// canonical 路径；`passthrough.*` 表示原样透传。
    pub canonical: &'static str,
    /// 兼容/转换说明，写进 gap report。
    pub note: &'static str,
    /// 是否整棵子树原样搬运。
    pub subtree: bool,
}

const fn leaf(wire: &'static str, canonical: &'static str, note: &'static str) -> Mapping {
    Mapping {
        wire,
        canonical,
        note,
        subtree: false,
    }
}

const fn subtree(wire: &'static str, canonical: &'static str, note: &'static str) -> Mapping {
    Mapping {
        wire,
        canonical,
        note,
        subtree: true,
    }
}

/// 一个协议的映射表与“canonical 必须有来源”的字段清单。
pub struct ProtocolSpec {
    pub protocol: IngressProtocol,
    pub mappings: &'static [Mapping],
    /// 该协议**应当**能填上的 canonical 路径；填不上说明协议能力缺失，必须显式记录。
    pub canonical_required: &'static [&'static str],
}

/// 单个 fixture 的探针结果。
#[derive(Debug, Clone, Default)]
pub struct CoverageReport {
    pub total_leaves: usize,
    pub mapped_leaves: usize,
    /// 没有映射的 wire 路径（稳定排序）。
    pub unmapped: Vec<String>,
    /// 已声明但没有 wire 来源的 canonical 路径（稳定排序）。
    pub missing_canonical: Vec<String>,
}

impl CoverageReport {
    pub fn ratio(&self) -> f64 {
        if self.total_leaves == 0 {
            return 1.0;
        }
        self.mapped_leaves as f64 / self.total_leaves as f64
    }
}

/// 取协议规格。
pub fn spec(protocol: IngressProtocol) -> &'static ProtocolSpec {
    match protocol {
        IngressProtocol::ChatCompletions => &CHAT,
        IngressProtocol::AnthropicMessages => &ANTHROPIC,
        IngressProtocol::Responses => &RESPONSES,
    }
}

/// 对一份 wire JSON 做覆盖探针。
pub fn probe(protocol: IngressProtocol, value: &Value) -> CoverageReport {
    let spec = spec(protocol);
    let mut leaves = BTreeSet::new();
    walk(value, "", &mut leaves);

    let mut report = CoverageReport {
        total_leaves: leaves.len(),
        ..CoverageReport::default()
    };

    for path in &leaves {
        if spec.mappings.iter().any(|m| covers(m, path)) {
            report.mapped_leaves += 1;
        } else {
            report.unmapped.push(path.clone());
        }
    }

    for required in spec.canonical_required {
        if !spec
            .mappings
            .iter()
            .any(|m| m.canonical.starts_with(required))
        {
            report.missing_canonical.push((*required).to_string());
        }
    }

    report
}

/// 映射是否覆盖该路径。
fn covers(mapping: &Mapping, path: &str) -> bool {
    if path == mapping.wire {
        return true;
    }
    if !mapping.subtree {
        return false;
    }
    path.strip_prefix(mapping.wire)
        .is_some_and(|rest| rest.starts_with('.'))
}

/// 收集叶子路径。空对象/空数组算一个叶子。
fn walk(value: &Value, path: &str, out: &mut BTreeSet<String>) {
    let join = |key: &str| {
        if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        }
    };

    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                walk(child, &join(key), out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            let array_path = format!("{path}[]");
            if items.iter().all(Value::is_object) {
                for item in items {
                    walk(item, &array_path, out);
                }
            } else {
                out.insert(array_path);
            }
        }
        _ => {
            out.insert(path.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Chat Completions
// ---------------------------------------------------------------------------

static CHAT: ProtocolSpec = ProtocolSpec {
    protocol: IngressProtocol::ChatCompletions,
    mappings: &[
        leaf("model", "meta.virtual_model", "虚拟模型 id"),
        leaf("messages[].role", "messages[].role", ""),
        leaf(
            "messages[].content",
            "messages[].blocks",
            "字符串 → 单个 Text block",
        ),
        leaf("messages[].content[].type", "messages[].blocks[].type", ""),
        leaf("messages[].content[].text", "messages[].blocks[].text", ""),
        leaf(
            "messages[].content[].image_url.url",
            "messages[].blocks[].source.url",
            "",
        ),
        leaf(
            "messages[].content[].image_url.detail",
            "messages[].blocks[].detail",
            "",
        ),
        leaf("messages[].name", "messages[].name", ""),
        leaf("messages[].tool_call_id", "messages[].tool_call_id", ""),
        leaf("messages[].tool_calls[].id", "messages[].blocks[].id", ""),
        leaf(
            "messages[].tool_calls[].type",
            "messages[].blocks[].type",
            "仅 function",
        ),
        leaf(
            "messages[].tool_calls[].function.name",
            "messages[].blocks[].name",
            "",
        ),
        leaf(
            "messages[].tool_calls[].function.arguments",
            "messages[].blocks[].arguments",
            "JSON 字符串 → Value",
        ),
        leaf("tools[].type", "tools[].kind", "仅 function"),
        leaf("tools[].function.name", "tools[].name", ""),
        leaf("tools[].function.description", "tools[].description", ""),
        subtree(
            "tools[].function.parameters",
            "tools[].parameters",
            "JSON Schema 原样保留",
        ),
        leaf("tools[].function.strict", "tools[].strict", ""),
        leaf("tool_choice", "output.tool_choice", "字符串或对象"),
        leaf("parallel_tool_calls", "output.parallel_tool_calls", ""),
        leaf("temperature", "sampling.temperature", ""),
        leaf("top_p", "sampling.top_p", ""),
        leaf("top_k", "sampling.top_k", ""),
        leaf("seed", "sampling.seed", ""),
        leaf("stop", "sampling.stop", ""),
        leaf("stop[]", "sampling.stop[]", ""),
        leaf("presence_penalty", "sampling.presence_penalty", ""),
        leaf("frequency_penalty", "sampling.frequency_penalty", ""),
        leaf("logprobs", "sampling.logprobs", ""),
        leaf("top_logprobs", "sampling.top_logprobs", ""),
        subtree(
            "logit_bias",
            "passthrough.logit_bias",
            "不在 canonical 解析",
        ),
        leaf("max_tokens", "output.max_tokens", ""),
        leaf("max_completion_tokens", "output.max_tokens", "新名优先"),
        leaf("n", "output.n", ""),
        subtree("response_format", "output.response_format", ""),
        leaf("stream", "output.stream", ""),
        leaf(
            "stream_options.include_usage",
            "output.stream_options.include_usage",
            "",
        ),
        leaf("reasoning_effort", "reasoning.effort", ""),
        leaf("user", "passthrough.user", "只审计，不参与路由"),
        subtree("metadata", "passthrough.metadata", "透传"),
        subtree("service_tier", "passthrough.service_tier", "透传"),
    ],
    canonical_required: &[
        "meta.virtual_model",
        "messages[].role",
        "messages[].blocks",
        "tools[]",
        "sampling.temperature",
        "output.max_tokens",
        "output.stream",
    ],
};

// ---------------------------------------------------------------------------
// Anthropic Messages
// ---------------------------------------------------------------------------

static ANTHROPIC: ProtocolSpec = ProtocolSpec {
    protocol: IngressProtocol::AnthropicMessages,
    mappings: &[
        leaf("model", "meta.virtual_model", "虚拟模型 id"),
        leaf("max_tokens", "output.max_tokens", "Anthropic 必填"),
        leaf("stream", "output.stream", ""),
        leaf("system", "system[]", "字符串 → 单个 SystemBlock"),
        leaf("system[].type", "system[].type", "仅 text"),
        leaf("system[].text", "system[].text", ""),
        subtree("system[].cache_control", "system[].cache_control", ""),
        leaf("messages[].role", "messages[].role", ""),
        leaf(
            "messages[].content",
            "messages[].blocks",
            "字符串 → 单个 Text block",
        ),
        leaf("messages[].content[].type", "messages[].blocks[].type", ""),
        leaf("messages[].content[].text", "messages[].blocks[].text", ""),
        subtree(
            "messages[].content[].cache_control",
            "messages[].blocks[].cache_control",
            "",
        ),
        leaf(
            "messages[].content[].source.type",
            "messages[].blocks[].source.kind",
            "base64 / url",
        ),
        leaf(
            "messages[].content[].source.media_type",
            "messages[].blocks[].source.media_type",
            "",
        ),
        leaf(
            "messages[].content[].source.data",
            "messages[].blocks[].source.data",
            "",
        ),
        leaf(
            "messages[].content[].source.url",
            "messages[].blocks[].source.url",
            "",
        ),
        leaf("messages[].content[].id", "messages[].blocks[].id", ""),
        leaf("messages[].content[].name", "messages[].blocks[].name", ""),
        subtree(
            "messages[].content[].input",
            "messages[].blocks[].arguments",
            "tool_use.input 即参数对象",
        ),
        leaf(
            "messages[].content[].tool_use_id",
            "messages[].blocks[].tool_use_id",
            "",
        ),
        leaf(
            "messages[].content[].is_error",
            "messages[].blocks[].is_error",
            "",
        ),
        leaf(
            "messages[].content[].content",
            "messages[].blocks[].content",
            "tool_result 内容",
        ),
        leaf(
            "messages[].content[].thinking",
            "messages[].blocks[].text",
            "thinking → Reasoning.text",
        ),
        leaf(
            "messages[].content[].signature",
            "messages[].blocks[].signature",
            "",
        ),
        leaf(
            "messages[].content[].data",
            "messages[].blocks[].signature",
            "redacted_thinking.data → signature, redacted=true",
        ),
        leaf("tools[].name", "tools[].name", ""),
        leaf("tools[].description", "tools[].description", ""),
        subtree(
            "tools[].input_schema",
            "tools[].parameters",
            "JSON Schema 原样保留",
        ),
        subtree("tools[].cache_control", "tools[].cache_control", ""),
        leaf(
            "tool_choice.type",
            "output.tool_choice.kind",
            "auto/any/tool",
        ),
        leaf("tool_choice.name", "output.tool_choice.name", ""),
        leaf(
            "tool_choice.disable_parallel_tool_use",
            "output.parallel_tool_calls",
            "取反",
        ),
        leaf("thinking.type", "reasoning.enabled", "enabled/disabled"),
        leaf("thinking.budget_tokens", "reasoning.budget_tokens", ""),
        leaf("temperature", "sampling.temperature", ""),
        leaf("top_p", "sampling.top_p", ""),
        leaf("top_k", "sampling.top_k", ""),
        leaf("stop_sequences", "sampling.stop", ""),
        leaf("stop_sequences[]", "sampling.stop[]", ""),
        leaf("metadata.user_id", "passthrough.user", "只审计"),
        subtree("service_tier", "passthrough.service_tier", "透传"),
    ],
    canonical_required: &[
        "meta.virtual_model",
        "system[]",
        "messages[].role",
        "messages[].blocks",
        "tools[]",
        "output.max_tokens",
        "reasoning.budget_tokens",
        "sampling.stop",
    ],
};

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

static RESPONSES: ProtocolSpec = ProtocolSpec {
    protocol: IngressProtocol::Responses,
    mappings: &[
        leaf("model", "meta.virtual_model", "虚拟模型 id"),
        leaf("instructions", "system[]", "字符串 → 单个 SystemBlock"),
        leaf("input", "messages[]", "字符串 → 单条 user 消息"),
        leaf("input[].role", "messages[].role", ""),
        leaf(
            "input[].content",
            "messages[].blocks",
            "字符串 → 单个 Text block",
        ),
        leaf("input[].content[].type", "messages[].blocks[].type", ""),
        leaf("input[].content[].text", "messages[].blocks[].text", ""),
        leaf(
            "input[].content[].image_url",
            "messages[].blocks[].source.url",
            "input_image",
        ),
        leaf(
            "input[].type",
            "messages[].blocks[].type",
            "message/function_call/function_call_output",
        ),
        leaf("input[].call_id", "messages[].blocks[].tool_use_id", ""),
        leaf("input[].id", "messages[].blocks[].id", ""),
        leaf("input[].name", "messages[].blocks[].name", ""),
        leaf(
            "input[].arguments",
            "messages[].blocks[].arguments",
            "JSON 字符串 → Value",
        ),
        leaf(
            "input[].output",
            "messages[].blocks[].content",
            "function_call_output",
        ),
        leaf("input[].status", "messages[].blocks[].status", "审计字段"),
        leaf("tools[].type", "tools[].kind", "仅 function"),
        leaf("tools[].name", "tools[].name", ""),
        leaf("tools[].description", "tools[].description", ""),
        subtree(
            "tools[].parameters",
            "tools[].parameters",
            "JSON Schema 原样保留",
        ),
        leaf("tools[].strict", "tools[].strict", ""),
        leaf("tool_choice", "output.tool_choice", ""),
        leaf("parallel_tool_calls", "output.parallel_tool_calls", ""),
        leaf("max_output_tokens", "output.max_tokens", ""),
        leaf("temperature", "sampling.temperature", ""),
        leaf("top_p", "sampling.top_p", ""),
        leaf("stream", "output.stream", ""),
        leaf("store", "passthrough.store", ""),
        leaf(
            "previous_response_id",
            "meta.previous_response_id",
            "协议状态，非会话历史",
        ),
        leaf("reasoning.effort", "reasoning.effort", ""),
        subtree("reasoning", "reasoning", "summary/effort 透传"),
        leaf("text.format.type", "output.response_format.kind", ""),
        leaf("text.format.name", "output.response_format.name", ""),
        leaf("text.format.strict", "output.response_format.strict", ""),
        subtree(
            "text.format.schema",
            "output.response_format.schema",
            "JSON Schema 原样保留",
        ),
        subtree("text", "output.response_format", ""),
        subtree("metadata", "passthrough.metadata", "透传"),
        leaf("truncation", "passthrough.truncation", ""),
        leaf("user", "passthrough.user", "只审计"),
    ],
    canonical_required: &[
        "meta.virtual_model",
        "messages[]",
        "tools[]",
        "output.max_tokens",
        "output.response_format",
        "reasoning.effort",
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn leaf_paths_use_array_markers() {
        let value = json!({
            "messages": [
                {"role": "user", "content": "hi"}
            ],
            "stop": ["a", "b"],
            "empty": {}
        });
        let mut leaves = BTreeSet::new();
        walk(&value, "", &mut leaves);
        let leaves: Vec<_> = leaves.into_iter().collect();
        assert_eq!(
            leaves,
            vec!["empty", "messages[].content", "messages[].role", "stop[]"]
        );
    }

    #[test]
    fn subtree_mapping_covers_schema_internals() {
        let value = json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "f",
                    "parameters": {"type": "object", "properties": {"a": {"type": "string"}}}
                }
            }]
        });
        let report = probe(IngressProtocol::ChatCompletions, &value);
        assert!(report.unmapped.is_empty(), "{:?}", report.unmapped);
        assert!((report.ratio() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_field_is_reported_not_ignored() {
        let value = json!({"model": "auto", "vendor_extension": {"x": 1}});
        let report = probe(IngressProtocol::ChatCompletions, &value);
        assert_eq!(report.unmapped, vec!["vendor_extension.x"]);
        assert!((report.ratio() - 0.5).abs() < f64::EPSILON);
    }
}
