//! S0 fixtures 自检：字段覆盖、错误分类、流式事件序列（§13 S0 门禁）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use tr_proto::fixture::{check, load_dir};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

#[test]
fn every_fixture_is_fully_mapped_and_parses() {
    let fixtures = load_dir(&fixtures_dir()).expect("加载 fixtures");
    assert!(
        fixtures.len() >= 18,
        "fixtures 数量不足：{}（三协议各 ≥6）",
        fixtures.len()
    );

    let mut failures = Vec::new();
    for fixture in &fixtures {
        let result = check(fixture);
        if !result.problems.is_empty() {
            failures.push(format!("{}: {}", fixture.id, result.problems.join("; ")));
        }
        assert!(
            (result.coverage.ratio() - 1.0).abs() < f64::EPSILON,
            "{}: 覆盖率 {}，未映射 {:?}",
            fixture.id,
            result.coverage.ratio(),
            result.coverage.unmapped
        );
    }

    assert!(
        failures.is_empty(),
        "fixture 自检失败:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_protocol_has_at_least_six_fixtures() {
    let fixtures = load_dir(&fixtures_dir()).expect("加载 fixtures");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for fixture in &fixtures {
        *counts.entry(fixture.protocol.clone()).or_default() += 1;
    }
    for protocol in ["chat_completions", "anthropic_messages", "responses"] {
        let count = counts.get(protocol).copied().unwrap_or(0);
        assert!(count >= 6, "{protocol} 只有 {count} 个 fixture");
    }
}
