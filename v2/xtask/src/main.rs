//! v2 构建与门禁任务。
//!
//! 用法：
//! ```text
//! cargo xtask check-deps      # 依赖方向门禁（§3.2 / §18.4.3）
//! cargo xtask fixtures-check  # S0 协议覆盖 + 流式事件门禁（§13 S0）
//! cargo xtask help
//! ```
//!
//! 为什么不用 clippy 自定义 lint：依赖方向是**架构约束**，需要可读的违规信息
//! （`crate -> dep` + 设计文档章节号），而不是一条编号 warning。

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 内部 crate 允许依赖的内部 crate 白名单（§3.2）。
///
/// `xtask` 是构建工具，允许依赖契约与协议实现来做门禁，不参与运行时依赖图。
const ALLOWED: &[(&str, &[&str])] = &[
    ("tr-contracts", &[]),
    ("tr-proto", &["tr-contracts"]),
    ("tr-core", &["tr-contracts"]),
    ("tr-decide", &["tr-contracts"]),
    ("tr-upstream", &["tr-contracts", "tr-proto"]),
    (
        "tr-agent",
        &[
            "tr-contracts",
            "tr-core",
            "tr-decide",
            "tr-upstream",
            // 仅 dev-dependency：测试用本地 mock 上游。
            "mock-upstream",
        ],
    ),
    (
        "tr-server",
        &[
            "tr-contracts",
            "tr-proto",
            "tr-agent",
            "tr-control",
            "mock-upstream",
        ],
    ),
    ("tr-control", &["tr-contracts", "tr-agent"]),
    (
        "tr-cli",
        &["tr-contracts", "tr-proto", "tr-core", "tr-agent"],
    ),
    ("mock-upstream", &[]),
    ("xtask", &["tr-proto"]),
];

/// `tr-core` 禁止出现的外部依赖：一旦出现就失去“纯函数可回放”。
const CORE_FORBIDDEN_DEPS: &[&str] = &["tokio", "reqwest", "rusqlite", "axum", "tower", "hyper"];

/// `tr-contracts` 允许的外部依赖：保持纯类型。
const CONTRACTS_ALLOWED_DEPS: &[&str] = &["serde", "serde_json", "sha2", "uuid", "thiserror"];

fn main() {
    let task = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".to_string());
    let code = match task.as_str() {
        "check-deps" => check_deps(),
        "fixtures-check" => fixtures_check(),
        "help" | "--help" | "-h" => {
            print_help();
            0
        }
        other => {
            eprintln!("unknown xtask: {other}");
            print_help();
            2
        }
    };
    std::process::exit(code);
}

fn print_help() {
    println!(
        "xtask 可用任务:\n\
         \x20 check-deps      依赖方向门禁（§3.2）\n\
         \x20 fixtures-check  S0 协议覆盖 + 流式事件门禁（默认阈值 95%）\n\
         \x20                 --fixtures <dir> --report <path> --min-coverage <0..1>\n\
         \x20 help            显示本帮助"
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 必须位于 v2/xtask")
        .to_path_buf()
}

fn check_deps() -> i32 {
    let root = workspace_root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = match Command::new(&cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(&root)
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            eprintln!("failed to run `{cargo} metadata`: {err}");
            return 2;
        }
    };
    if !output.status.success() {
        eprintln!(
            "`cargo metadata` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return 2;
    }

    let metadata: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("cannot parse cargo metadata: {err}");
            return 2;
        }
    };

    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata 缺少 packages");

    let internal: BTreeSet<&str> = ALLOWED.iter().map(|(name, _)| *name).collect();
    let mut declared_internal: BTreeSet<String> = BTreeSet::new();
    let mut violations: Vec<String> = Vec::new();
    let mut edge_count = 0usize;

    for package in packages {
        let name = package["name"].as_str().unwrap_or_default();
        // 只检查 workspace 成员（path 依赖，无 source）。
        if !package["source"].is_null() {
            continue;
        }
        if internal.contains(name) {
            declared_internal.insert(name.to_string());
        }

        for dep in package["dependencies"].as_array().into_iter().flatten() {
            let Some(dep_name) = dep["name"].as_str() else {
                continue;
            };
            let kind = dep["kind"].as_str().unwrap_or("normal");

            if internal.contains(dep_name) {
                edge_count += 1;
                let allowed = ALLOWED
                    .iter()
                    .find(|(crate_name, _)| *crate_name == name)
                    .map(|(_, allowed)| allowed.contains(&dep_name))
                    .unwrap_or(false);
                if !allowed {
                    violations.push(format!(
                        "{name} -> {dep_name} ({kind}): 不在 §3.2 允许方向内"
                    ));
                }
            }

            if name == "tr-core" && CORE_FORBIDDEN_DEPS.contains(&dep_name) {
                violations.push(format!(
                    "tr-core -> {dep_name} ({kind}): `core` 必须零 IO（§3.2 规则 1）"
                ));
            }

            if name == "tr-contracts" && !CONTRACTS_ALLOWED_DEPS.contains(&dep_name) {
                violations.push(format!(
                    "tr-contracts -> {dep_name} ({kind}): `contracts` 只允许纯类型依赖（§3.2）"
                ));
            }
        }
    }

    for (name, _) in ALLOWED {
        if !declared_internal.contains(*name) {
            violations.push(format!(
                "缺少 crate `{name}`：白名单里的 crate 必须在 workspace 中存在"
            ));
        }
    }

    // core 测试里也不允许 async 运行时（否则回放不再是纯函数）。
    let core_dir = root.join("crates/core");
    for path in rust_files(&core_dir) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // 只看代码行：文档注释里提到属性名是合法的。
        let uses_runtime = text.lines().any(|line| {
            let code = line.trim_start();
            !code.starts_with("//")
                && (code.contains("#[tokio::test]") || code.contains("#[tokio::main]"))
        });
        if uses_runtime {
            violations.push(format!(
                "{}: `core` 测试不得使用 tokio 运行时（§3.2 规则 1）",
                path.display()
            ));
        }
    }

    if violations.is_empty() {
        println!(
            "check-deps OK: {} crates, {edge_count} internal edges",
            declared_internal.len()
        );
        0
    } else {
        eprintln!("check-deps FAILED:");
        for violation in &violations {
            eprintln!("  - {violation}");
        }
        1
    }
}

/// 递归收集 `.rs` 文件（不引入 `walkdir`，保持 xtask 零额外依赖）。
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files
}

#[derive(Default)]
struct ProtocolStats {
    fixtures: usize,
    streams: usize,
    errors: usize,
    leaves: usize,
    mapped: usize,
    unmapped: Vec<(String, String)>,
}

fn fixtures_check() -> i32 {
    let root = workspace_root();
    let args: Vec<String> = std::env::args().skip(2).collect();
    let fixtures_dir = flag_value(&args, "--fixtures")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("tests/fixtures"));
    let report_path = flag_value(&args, "--report")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("../docs/s0-gap-report.md"));
    let min_coverage = flag_value(&args, "--min-coverage")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.95);

    let fixtures = match tr_proto::fixture::load_dir(&fixtures_dir) {
        Ok(fixtures) => fixtures,
        Err(err) => {
            eprintln!(
                "cannot load fixtures from {}: {err}",
                fixtures_dir.display()
            );
            return 2;
        }
    };
    if fixtures.is_empty() {
        eprintln!("no fixtures in {}", fixtures_dir.display());
        return 2;
    }

    let mut stats: BTreeMap<String, ProtocolStats> = BTreeMap::new();
    let mut problems: Vec<String> = Vec::new();

    for fixture in &fixtures {
        let result = tr_proto::fixture::check(fixture);
        let entry = stats.entry(fixture.protocol.clone()).or_default();
        entry.fixtures += 1;
        entry.leaves += result.coverage.total_leaves;
        entry.mapped += result.coverage.mapped_leaves;
        if !fixture.chunks.is_empty() {
            entry.streams += 1;
        }
        if fixture.error.is_some() {
            entry.errors += 1;
        }
        for path in result.coverage.unmapped {
            entry.unmapped.push((fixture.id.clone(), path));
        }
        for problem in result.problems {
            problems.push(format!("{}: {problem}", fixture.id));
        }
    }

    for (protocol, entry) in &stats {
        let ratio = ratio(entry.mapped, entry.leaves);
        println!(
            "{protocol:20} fixtures={:2} streams={:2} errors={:2} leaves={:4} mapped={:4} coverage={:.1}%",
            entry.fixtures,
            entry.streams,
            entry.errors,
            entry.leaves,
            entry.mapped,
            ratio * 100.0
        );
        if ratio < min_coverage {
            problems.push(format!(
                "{protocol}: coverage {:.1}% < 阈值 {:.0}%",
                ratio * 100.0,
                min_coverage * 100.0
            ));
        }
        if !entry.unmapped.is_empty() {
            problems.push(format!(
                "{protocol}: {} 个未映射请求字段",
                entry.unmapped.len()
            ));
        }
    }

    if let Err(err) = write_gap_report(&report_path, &stats, &problems, min_coverage) {
        eprintln!("cannot write report {}: {err}", report_path.display());
        problems.push(format!("report write failed: {err}"));
    } else {
        println!("gap report: {}", report_path.display());
    }

    if problems.is_empty() {
        println!(
            "fixtures-check OK: {} fixtures across {} protocols",
            fixtures.len(),
            stats.len()
        );
        0
    } else {
        eprintln!("fixtures-check FAILED:");
        for problem in &problems {
            eprintln!("  - {problem}");
        }
        1
    }
}

fn ratio(mapped: usize, total: usize) -> f64 {
    if total == 0 {
        1.0
    } else {
        mapped as f64 / total as f64
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn write_gap_report(
    path: &Path,
    stats: &BTreeMap<String, ProtocolStats>,
    problems: &[String],
    min_coverage: f64,
) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("# S0 协议覆盖报告\n\n");
    out.push_str(
        "> 由 `cargo xtask fixtures-check --report docs/s0-gap-report.md` 生成，请勿手改。\n\
         > 覆盖探针只统计**请求**字段；响应形状与流式事件由 fixture 自检断言。\n\
         > 本报告不是“兼容承诺”：v2 的 canonical 面见设计文档 §4.1。\n\n",
    );
    out.push_str(
        "## 数据来源与已知限制\n\n\
         - 样本为**自建协议样本**（`v2/tests/fixtures/`），不是真实客户端录制；\n\
           因此 100% 覆盖率只证明“映射表覆盖了这些形状”，不证明真实客户端不会带来新字段。\n\
         - 三客户端冒烟（Hermes / Codex / Claude Code）尚未执行，需在具备客户端与上游凭据的\n\
           环境补录真实样本后重跑本门禁；届时新增字段会出现在“未映射字段”里。\n\
         - provider 私有字段靠 `passthrough` 兜底；探针里映射到 `passthrough.*` 的字段视为“有处置”。\n\
         - mock upstream / v1 重放尚未接入：P0a 的 e2e 需要它，届时补 `crates/mock-upstream`。\n\n",
    );
    out.push_str("## 覆盖率\n\n");
    out.push_str("| 协议 | fixtures | 流式 | 错误 | wire 叶子 | 已映射 | 覆盖率 |\n");
    out.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
    for (protocol, entry) in stats {
        out.push_str(&format!(
            "| {protocol} | {} | {} | {} | {} | {} | {:.1}% |\n",
            entry.fixtures,
            entry.streams,
            entry.errors,
            entry.leaves,
            entry.mapped,
            ratio(entry.mapped, entry.leaves) * 100.0
        ));
    }

    out.push_str("\n## 未映射字段\n\n");
    let mut any_unmapped = false;
    for (protocol, entry) in stats {
        for (fixture, field) in &entry.unmapped {
            any_unmapped = true;
            out.push_str(&format!("- `{protocol}` / `{fixture}`: `{field}`\n"));
        }
    }
    if !any_unmapped {
        out.push_str("（无）\n");
    }

    out.push_str("\n## 断言失败\n\n");
    if problems.is_empty() {
        out.push_str("（无）\n");
    } else {
        for problem in problems {
            out.push_str(&format!("- {problem}\n"));
        }
    }

    out.push_str(&format!(
        "\n门禁阈值：每个协议覆盖率 ≥ {:.0}%。\n",
        min_coverage * 100.0
    ));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, out)
}
