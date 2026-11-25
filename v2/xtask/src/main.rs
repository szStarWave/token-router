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
            // v3 P5-A：file: 相对路径凭据按数据目录解析（sidecar cwd 不可假设）。
            "tr-upstream",
            "mock-upstream",
        ],
    ),
    (
        "tr-control",
        &["tr-contracts", "tr-agent", "tr-onboard", "tr-upstream"],
    ),
    (
        "tr-cli",
        &[
            "tr-contracts",
            "tr-proto",
            "tr-core",
            "tr-agent",
            // P4-045：admin 子命令通过 tr-client 走 HTTP 管理面（不直连 SQLite）。
            "tr-client",
        ],
    ),
    ("tr-client", &["tr-contracts"]),
    ("tr-onboard", &[]),
    ("mock-upstream", &[]),
    ("xtask", &["tr-proto", "tr-agent"]),
];

/// `tr-core` 禁止出现的外部依赖：一旦出现就失去“纯函数可回放”。
const CORE_FORBIDDEN_DEPS: &[&str] = &["tokio", "reqwest", "rusqlite", "axum", "tower", "hyper"];

/// `tr-contracts` 允许的外部依赖：保持纯类型。
const CONTRACTS_ALLOWED_DEPS: &[&str] =
    &["serde", "serde_json", "sha2", "uuid", "thiserror", "ts-rs"];

fn main() {
    let task = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".to_string());
    let code = match task.as_str() {
        "check-deps" => check_deps(),
        "fixtures-check" => fixtures_check(),
        "config-only-check" => config_only_check(),
        "contract-check" => contract_check(),
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
         \x20 config-only-check  P3-18：只改 TOML 新增候选/虚拟模型，core diff = 0\n\
         \x20 contract-check   P4-003：契约 DTO → TS 类型生成物无 diff\n\
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

/// P3-18：证明“新增候选/虚拟模型只改 TOML，`core` diff = 0”。
/// P4-003：契约生成门禁。
///
/// 重新生成 `tr-contracts` 的 TS 绑定并与仓库中的生成物比较：
/// 任何人改了 admin DTO 但没有重新导出类型，此门禁立即失败。
/// 生成的临时目录与仓库内 `bindings/` 逐一对比，不污染工作区。
fn contract_check() -> i32 {
    let root = workspace_root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    let temp = match std::env::temp_dir().join("tr-contract-check").to_str() {
        Some(path) => path.to_string(),
        None => {
            eprintln!("cannot create temp path");
            return 2;
        }
    };
    let _ = std::fs::remove_dir_all(&temp);

    // 在临时目录重新导出（TS_RS_EXPORT_DIR 控制导出位置）。
    let output = match Command::new(&cargo)
        .args(["test", "-p", "tr-contracts", "--lib", "export_bindings"])
        .env("TS_RS_EXPORT_DIR", &temp)
        .env("TS_RS_LARGE_INT", "number")
        .current_dir(&root)
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            eprintln!("failed to run `cargo test`: {err}");
            return 2;
        }
    };
    if !output.status.success() {
        eprintln!(
            "binding export failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return 1;
    }

    let committed = root.join("crates/contracts/bindings");
    let status = diff_trees(&committed, Path::new(&temp));
    if status.is_empty() {
        println!("contract-check OK: TS bindings up to date");
        0
    } else {
        eprintln!(
            "contract-check FAILED: TS bindings out of date ({} file(s)):",
            status.len()
        );
        for entry in &status {
            eprintln!("  {entry}");
        }
        eprintln!(
            "运行 `cargo test -p tr-contracts --lib export_bindings` 并提交 crates/contracts/bindings"
        );
        1
    }
}

/// 递归比较两个目录；返回差异描述列表（缺失/多余/内容不同）。
fn diff_trees(left: &Path, right: &Path) -> Vec<String> {
    let mut diffs = Vec::new();
    let left_files = collect_binding_files(left);
    let right_files = collect_binding_files(right);
    for path in &left_files {
        let relative = path.strip_prefix(left).unwrap_or(path);
        let counter = right.join(relative);
        if !counter.exists() {
            diffs.push(format!("missing in regenerated: {}", relative.display()));
        } else if std::fs::read(path).unwrap_or_default()
            != std::fs::read(&counter).unwrap_or_default()
        {
            diffs.push(format!("content differs: {}", relative.display()));
        }
    }
    for path in &right_files {
        let relative = path.strip_prefix(right).unwrap_or(path);
        if !left.join(relative).exists() {
            diffs.push(format!("unexpected new file: {}", relative.display()));
        }
    }
    diffs.sort();
    diffs
}

fn collect_binding_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

///
/// 做法：
/// 1. 对 `crates/core/src` 与 `crates/contracts/src` 取内容指纹；
/// 2. 在示例配置上**纯文本追加**一个新候选与一个新虚拟模型；
/// 3. 用真实配置加载器校验结果；
/// 4. 重新取指纹，必须与步骤 1 相同（检查过程没有改内核）。
fn config_only_check() -> i32 {
    let root = workspace_root();
    let example = root.join("example/config.minimal.toml");
    let baseline = match std::fs::read_to_string(&example) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("failed to read {}: {error}", example.display());
            return 2;
        }
    };

    let before = match kernel_digest(&root) {
        Ok(digest) => digest,
        Err(error) => {
            eprintln!("failed to hash kernel sources: {error}");
            return 2;
        }
    };

    let baseline_catalog =
        match tr_agent::config::ConfigFile::parse(&baseline).and_then(|file| file.into_catalog()) {
            Ok(catalog) => catalog,
            Err(error) => {
                eprintln!("example config is invalid: {error}");
                return 1;
            }
        };
    let (base_candidates, base_models) = (
        baseline_catalog.candidates.len(),
        baseline_catalog.virtual_models.len(),
    );

    // 纯配置追加：不碰 Rust 源码。
    let additions = r#"

# config-only-check 追加：一个新候选 + 一个新虚拟模型。
[[candidates]]
id = "config-only-candidate"
tier = "cloud"
strength = 90
provider = "openai"
base_url = "https://config-only.example.com/v1"
model = "config-only-model"
window = 200000
input = ["text"]

[[virtual_models]]
provider = "router"
id = "config-only"
name = "Config Only"
profile = "default"
candidate_pool = ["config-only-candidate"]
"#;
    let toml = format!("{baseline}{additions}");

    let temp = std::env::temp_dir().join(format!(
        "tr-config-only-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    if let Err(error) = std::fs::write(&temp, &toml) {
        eprintln!("failed to write temporary config: {error}");
        return 2;
    }

    let catalog = match tr_agent::config::load(&temp) {
        Ok(catalog) => catalog,
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            eprintln!("config-only additions were rejected: {error}");
            return 1;
        }
    };
    let _ = std::fs::remove_file(&temp);

    let mut failures = Vec::new();
    if catalog.candidates.len() != base_candidates + 1 {
        failures.push(format!(
            "expected {} candidates after addition, got {}",
            base_candidates + 1,
            catalog.candidates.len()
        ));
    }
    if catalog.virtual_models.len() != base_models + 1 {
        failures.push(format!(
            "expected {} virtual models after addition, got {}",
            base_models + 1,
            catalog.virtual_models.len()
        ));
    }
    match catalog.candidate("config-only-candidate") {
        Some(candidate) if candidate.upstream.model == "config-only-model" => {}
        _ => failures.push("added candidate did not resolve".to_string()),
    }
    match catalog.virtual_model("config-only") {
        Some(model)
            if model.candidate_pool == vec!["config-only-candidate".to_string()]
                && catalog.profile_for(model).is_some() => {}
        _ => failures.push("added virtual model did not resolve to a profile".to_string()),
    }

    let after = match kernel_digest(&root) {
        Ok(digest) => digest,
        Err(error) => {
            eprintln!("failed to hash kernel sources: {error}");
            return 2;
        }
    };
    if before != after {
        failures.push("kernel sources changed during config-only check".to_string());
    }

    if failures.is_empty() {
        println!(
            "config-only-check: core diff = 0; candidates {base_candidates} -> {}, virtual models {base_models} -> {} (TOML only)",
            base_candidates + 1,
            base_models + 1
        );
        0
    } else {
        eprintln!("config-only-check FAILED:");
        for failure in failures {
            eprintln!("- {failure}");
        }
        1
    }
}

/// `crates/core/src` + `crates/contracts/src` 的内容指纹（路径 + 字节）。
fn kernel_digest(root: &Path) -> std::io::Result<u64> {
    use std::hash::{Hash, Hasher};

    let mut files = Vec::new();
    for relative in ["crates/core/src", "crates/contracts/src"] {
        collect_files(&root.join(relative), &mut files)?;
    }
    files.sort();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for file in files {
        let relative = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .to_string();
        relative.hash(&mut hasher);
        std::fs::read(&file)?.hash(&mut hasher);
    }
    Ok(hasher.finish())
}

fn collect_files(directory: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
    Ok(())
}
