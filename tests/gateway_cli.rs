use std::path::PathBuf;
use std::process::Command;

fn router_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_token_router") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("target"));
    for profile in ["debug", "release"] {
        let candidate = target.join(profile).join("token-router");
        if candidate.exists() {
            return candidate;
        }
    }
    target.join("debug/token-router")
}

#[test]
fn gateway_subcommands_are_registered() {
    let out = Command::new(router_bin())
        .args(["gateway", "--help"])
        .output()
        .expect("run token-router gateway --help");

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let help = String::from_utf8_lossy(&out.stdout);
    for cmd in ["start", "stop", "status", "restart"] {
        assert!(help.contains(cmd), "missing subcommand `{cmd}` in:\n{help}");
    }
    assert!(!help.contains("  run"), "gateway run should be removed:\n{help}");
}

#[test]
fn stats_is_registered() {
    let out = Command::new(router_bin())
        .args(["stats", "--help"])
        .output()
        .expect("run token-router stats --help");

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("json"), "missing --json in:\n{help}");
    assert!(help.contains("global"), "missing --global in:\n{help}");
    assert!(help.contains("lang"), "missing --lang in:\n{help}");
}

#[test]
fn setup_is_registered() {
    let out = Command::new(router_bin())
        .args(["setup", "--help"])
        .output()
        .expect("run token-router setup --help");

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let help = String::from_utf8_lossy(&out.stdout);
    for flag in ["--remote", "--reset", "--non-interactive", "--cloud-url", "--edge-url"] {
        assert!(help.contains(flag), "missing `{flag}` in:\n{help}");
    }
}

#[test]
fn cli_accepts_home_and_port_flags() {
    let out = Command::new(router_bin())
        .arg("--help")
        .output()
        .expect("run token-router --help");

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("--home"), "missing --home in:\n{help}");
    assert!(help.contains("--port"), "missing --port in:\n{help}");
    assert!(
        !help.contains("--config"),
        "legacy --config should be removed:\n{help}"
    );
}

#[test]
fn env_reports_custom_home_paths() {
    let temp = std::env::temp_dir().join(format!("flowy-router-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).expect("create temp home");
    std::fs::write(
        temp.join("config.toml"),
        "[gateway]\nlisten = \"127.0.0.1:11080\"\n",
    )
    .expect("write temp config");

    let out = Command::new(router_bin())
        .args(["--home", &temp.display().to_string(), "env"])
        .output()
        .expect("run token-router env --home");

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(&format!("app_dir:        {}", temp.display())),
        "expected custom app_dir in:\n{text}"
    );
    assert!(
        text.contains(&format!("config_file:    {}", temp.join("config.toml").display())),
        "expected custom config_file in:\n{text}"
    );

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn env_prints_paths() {
    let out = Command::new(router_bin())
        .args(["env"])
        .output()
        .expect("run token-router env");

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    for key in ["user_home:", "config_file:", "gateway_log:", "gateway_url:"] {
        assert!(text.contains(key), "missing `{key}` in:\n{text}");
    }
}
