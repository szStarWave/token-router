//! WSL 支持（技术方案 §8.4）：检测发行版、UNC home、网关可达地址探测。
//!
//! 约束：
//! - 只支持 Windows 宿主；
//! - `wsl.exe` 一律参数数组调用，不拼字符串（防注入）；
//! - 发行版名白名单校验（字母/数字/连字符/点/下划线）。

use std::path::PathBuf;

use crate::{AgentKind, SetupError, Target};

/// 发行版名合法性：`^[A-Za-z0-9][A-Za-z0-9._-]*$`。
pub fn valid_distro_name(distro: &str) -> bool {
    let mut chars = distro.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// WSL 环境检测报告。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WslReport {
    pub wsl_available: bool,
    pub distros: Vec<String>,
    pub default_distro: Option<String>,
    pub error: Option<String>,
}

/// 枚举 WSL 发行版（`wsl -l -v` 输出解析）。
///
/// 非 Windows / wsl 不存在时返回 `wsl_available: false`，不报错。
pub fn detect() -> WslReport {
    let output = std::process::Command::new("wsl.exe")
        .args(["-l", "-v"])
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return WslReport {
                wsl_available: false,
                distros: Vec::new(),
                default_distro: None,
                error: Some(format!("wsl exited {}", output.status)),
            };
        }
        Err(error) => {
            return WslReport {
                wsl_available: false,
                distros: Vec::new(),
                default_distro: None,
                error: Some(error.to_string()),
            };
        }
    };

    // wsl.exe 输出是 UTF-16LE；做容错解码。
    let text = decode_wsl_output(&output.stdout);
    parse_distros(&text)
}

/// 从 `wsl -l -v` 文本解析发行版列表（第一列；带 `*` 的是默认）。
fn parse_distros(text: &str) -> WslReport {
    let mut distros = Vec::new();
    let mut default_distro = None;
    for line in text.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let is_default = line.starts_with('*');
        let first = line
            .trim_start_matches('*')
            .split_whitespace()
            .next()
            .unwrap_or("");
        if first.is_empty() || !valid_distro_name(first) {
            continue;
        }
        if is_default {
            default_distro = Some(first.to_string());
        }
        distros.push(first.to_string());
    }
    WslReport {
        wsl_available: !distros.is_empty(),
        distros,
        default_distro,
        error: None,
    }
}

/// wsl.exe 在 Windows 上输出 UTF-16LE；遇 NUL 字节按 UTF-16 解码，否则按 UTF-8。
fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.len() >= 2
        && bytes.len() % 2 == 0
        && bytes.iter().filter(|b| **b == 0).count() > bytes.len() / 4
    {
        // 粗判 UTF-16LE：NUL 多。每两个字节组成一个码元。
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

/// WSL 目标的 `$HOME`：UNC 路径 `\\wsl$\<distro>`（Win11 也接受 `\\wsl.localhost\<distro>`，
/// 先探测 wsl.localhost 再回退 wsl$）。
pub fn home_unc(distro: &str) -> Result<PathBuf, SetupError> {
    if !valid_distro_name(distro) {
        return Err(SetupError::Parse(format!("invalid distro name `{distro}`")));
    }
    for base in [r"\\wsl.localhost", r"\\wsl$"] {
        let candidate = PathBuf::from(format!("{base}\\{distro}"));
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    // 都不可达时仍返回 wsl$ 路径（探测失败在 detect 阶段已提示）。
    Ok(PathBuf::from(format!(r"\\wsl$\{distro}")))
}

/// 从 WSL 内探测网关可达地址：
/// WSL2 NAT 下 `127.0.0.1` 指向 WSL 自己，需用宿主在 WSL 里的地址
/// （默认网关）。返回 `http://<ip>:<port>`。
pub fn probe_gateway_url(distro: &str, host_port: u16) -> Result<String, SetupError> {
    if !valid_distro_name(distro) {
        return Err(SetupError::Parse(format!("invalid distro name `{distro}`")));
    }
    // 参数数组调用；命令固定，不拼接用户输入。
    let output = std::process::Command::new("wsl.exe")
        .args([
            "-d",
            distro,
            "--",
            "sh",
            "-c",
            "ip route show default | awk '{print $3}'",
        ])
        .output()
        .map_err(SetupError::from)?;
    let text = decode_wsl_output(&output.stdout);
    let gateway = text.trim().lines().next().unwrap_or("").trim();
    if gateway.is_empty() || !gateway.split('.').all(|octet| octet.parse::<u8>().is_ok()) {
        // 探测失败：回退 localhost（WSL1 mirrored 网络可用）。
        return Ok(format!("http://127.0.0.1:{host_port}"));
    }
    Ok(format!("http://{gateway}:{host_port}"))
}

/// WSL 目标下的 Agent 列表（§8.4：与 host 对齐，可少 hermes-flash）。
pub fn supported_agents() -> &'static [AgentKind] {
    AgentKind::ALL
}

/// 构造 WSL 目标（含白名单校验）。
pub fn target(distro: &str) -> Result<Target, SetupError> {
    if !valid_distro_name(distro) {
        return Err(SetupError::Parse(format!("invalid distro name `{distro}`")));
    }
    Ok(Target::Wsl {
        distro: distro.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distro_name_whitelist() {
        assert!(valid_distro_name("Ubuntu-22.04"));
        assert!(valid_distro_name("Debian"));
        assert!(valid_distro_name("arch_1"));
        assert!(!valid_distro_name("-leading"));
        assert!(!valid_distro_name("with space"));
        assert!(!valid_distro_name("a;rm -rf"));
        assert!(!valid_distro_name(""));
    }

    #[test]
    fn parse_wsl_list_output() {
        let text = "  NAME            STATE           VERSION\r\n* Ubuntu-22.04    Running         2\r\n  Debian          Stopped         2\r\n";
        let report = parse_distros(text);
        assert!(report.wsl_available);
        assert_eq!(report.distros, vec!["Ubuntu-22.04", "Debian"]);
        assert_eq!(report.default_distro.as_deref(), Some("Ubuntu-22.04"));
    }

    #[test]
    fn parse_wsl_list_empty() {
        let report = parse_distros("");
        assert!(!report.wsl_available);
        assert_eq!(report.default_distro, None);
    }

    #[test]
    fn utf16_decode() {
        // "Ubuntu" in UTF-16LE
        let bytes: Vec<u8> = "Ubuntu"
            .chars()
            .flat_map(|c| (c as u16).to_le_bytes())
            .collect();
        assert_eq!(decode_wsl_output(&bytes), "Ubuntu");
    }

    #[test]
    fn home_unc_builds_path() {
        let path = home_unc("Ubuntu-22.04").unwrap();
        assert!(path.to_string_lossy().contains("Ubuntu-22.04"));
        assert!(home_unc("bad name").is_err());
    }

    #[test]
    fn target_validates_name() {
        assert!(target("Ubuntu").is_ok());
        assert!(target("x;rm").is_err());
    }
}
