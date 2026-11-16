//! v2 构建与门禁任务。
//!
//! 用法：
//! ```text
//! cargo xtask check-deps    # 依赖方向门禁（§3.2 / §18.4.3）
//! cargo xtask help
//! ```
//!
//! 为什么不用 clippy 自定义 lint：依赖方向是**架构约束**，需要可读的违规信息
//! （`crate -> dep` + 设计文档章节号），而不是一条编号 warning。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 内部 crate 允许依赖的内部 crate 白名单（§3.2）。
const ALLOWED: &[(&str, &[&str])] = &[
    ("tr-contracts", &[]),
    ("tr-proto", &["tr-contracts"]),
    ("tr-core", &["tr-contracts"]),
    ("tr-decide", &["tr-contracts"]),
    ("tr-upstream", &["tr-contracts"]),
    (
        "tr-agent",
        &["tr-contracts", "tr-core", "tr-decide", "tr-upstream"],
    ),
    ("tr-server", &["tr-contracts", "tr-proto", "tr-agent"]),
    ("tr-control", &["tr-contracts", "tr-agent"]),
    (
        "tr-cli",
        &["tr-contracts", "tr-proto", "tr-core", "tr-agent"],
    ),
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
    println!("xtask 可用任务:\n  check-deps   依赖方向门禁（§3.2）\n  help         显示本帮助");
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
