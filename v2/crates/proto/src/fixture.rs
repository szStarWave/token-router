//! Fixture 加载与自检（S0-2/S0-4）。
//!
//! 仅供测试与 `xtask fixtures-check` 使用，不属于运行时路径。
//! fixture 是"协议形状"的事实来源：新增字段先加 fixture，再加映射。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use tr_contracts::{CanonicalStreamEvent, ErrorClass, IngressProtocol};

use crate::coverage::{CoverageReport, probe};
use crate::errors::classify;
use crate::sse::{RawChunk, canonicalize};

/// 一个 wire 侧样本。
#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub id: String,
    pub protocol: String,
    pub stream: bool,
    #[serde(default)]
    pub request: Value,
    #[serde(default)]
    pub response: Option<Value>,
    #[serde(default)]
    pub chunks: Vec<FixtureChunk>,
    #[serde(default)]
    pub error: Option<FixtureError>,
    /// 期望的 canonical 事件序列（流式样本）。
    #[serde(default)]
    pub expect_events: Vec<Value>,
    #[serde(default)]
    pub expect_error_class: Option<ErrorClass>,
    #[serde(default)]
    pub expect_retry_after_ms: Option<u64>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub stream_framing: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureChunk {
    #[serde(default)]
    pub event: Option<String>,
    pub data: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureError {
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Value,
}

impl Fixture {
    pub fn protocol_kind(&self) -> Result<IngressProtocol, String> {
        match self.protocol.as_str() {
            "chat_completions" => Ok(IngressProtocol::ChatCompletions),
            "anthropic_messages" => Ok(IngressProtocol::AnthropicMessages),
            "responses" => Ok(IngressProtocol::Responses),
            other => Err(format!("unknown protocol `{other}`")),
        }
    }

    pub fn raw_chunks(&self) -> Vec<RawChunk> {
        self.chunks
            .iter()
            .map(|chunk| RawChunk {
                event: chunk.event.clone(),
                data: chunk.data.clone(),
            })
            .collect()
    }
}

/// 自检结果。
#[derive(Debug, Clone)]
pub struct FixtureCheck {
    pub coverage: CoverageReport,
    pub problems: Vec<String>,
    pub events: Vec<CanonicalStreamEvent>,
}

/// 对单个 fixture 做覆盖探针 + 错误/流式断言。
pub fn check(fixture: &Fixture) -> FixtureCheck {
    let mut problems = Vec::new();
    let protocol = match fixture.protocol_kind() {
        Ok(protocol) => protocol,
        Err(err) => {
            return FixtureCheck {
                coverage: CoverageReport::default(),
                problems: vec![err],
                events: Vec::new(),
            };
        }
    };

    let coverage = probe(protocol, &fixture.request);
    if !coverage.unmapped.is_empty() {
        problems.push(format!("unmapped request fields: {:?}", coverage.unmapped));
    }
    if !coverage.missing_canonical.is_empty() {
        problems.push(format!(
            "canonical fields without wire source: {:?}",
            coverage.missing_canonical
        ));
    }

    let mut events = Vec::new();
    if let Some(error) = &fixture.error {
        let classified = classify(error.status, &error.headers, &error.body);
        if let Some(expected) = fixture.expect_error_class
            && classified.class != expected
        {
            problems.push(format!(
                "error class: expected {expected:?}, got {:?}",
                classified.class
            ));
        }
        if let Some(expected) = fixture.expect_retry_after_ms
            && classified.retry_after_ms != Some(expected)
        {
            problems.push(format!(
                "retry-after: expected {expected}ms, got {:?}",
                classified.retry_after_ms
            ));
        }
    }

    if !fixture.chunks.is_empty() {
        match canonicalize(protocol, &fixture.raw_chunks()) {
            Ok(parsed) => {
                let expected: Vec<CanonicalStreamEvent> = fixture
                    .expect_events
                    .iter()
                    .map(|value| {
                        serde_json::from_value(value.clone()).expect("expect_events 形状非法")
                    })
                    .collect();
                if parsed != expected {
                    problems.push(format!(
                        "stream events mismatch:\n  expected: {expected:?}\n  parsed:   {parsed:?}"
                    ));
                }
                events = parsed;
            }
            Err(err) => problems.push(format!("stream parse failed: {err}")),
        }
    }

    FixtureCheck {
        coverage,
        problems,
        events,
    }
}

/// 加载目录下所有 fixture（递归，按 id 排序）。
pub fn load_dir(dir: &Path) -> Result<Vec<Fixture>, String> {
    let mut paths = Vec::new();
    collect_json(dir, &mut paths);
    paths.sort();

    let mut fixtures = Vec::new();
    let mut seen = BTreeMap::new();
    for path in paths {
        let text =
            std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let fixture: Fixture =
            serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))?;
        if let Some(previous) = seen.insert(fixture.id.clone(), path.clone()) {
            return Err(format!(
                "duplicate fixture id `{}` in {} and {}",
                fixture.id,
                previous.display(),
                path.display()
            ));
        }
        fixtures.push(fixture);
    }
    fixtures.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(fixtures)
}

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
}
