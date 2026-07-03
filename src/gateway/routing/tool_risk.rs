use crate::gateway::api::openai::ToolCall;

/// Result of classifying pending tool calls on the latest assistant turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolRiskAssessment {
    /// Delete/move file operations — hard gate to cloud.
    pub risky_tool_hard: bool,
    /// browser / sessions_spawn / message — difficulty bias only.
    pub risky_tool_soft: bool,
    pub risky_tool_names: Vec<String>,
    pub risky_tool_hard_names: Vec<String>,
    pub risky_tool_soft_names: Vec<String>,
}

pub fn assess_tool_calls(calls: &[ToolCall]) -> ToolRiskAssessment {
    let mut hard_names = Vec::new();
    let mut soft_names = Vec::new();

    for call in calls {
        match classify_tool_call(&call.function.name, &call.function.arguments) {
            ToolTier::Hard => {
                let label = hard_label(&call.function.name, &call.function.arguments);
                push_unique(&mut hard_names, &label);
            }
            ToolTier::Soft => {
                if let Some(canonical) = soft_canonical_name(&call.function.name) {
                    push_unique(&mut soft_names, canonical);
                }
            }
            ToolTier::None => {}
        }
    }

    let mut risky_tool_names = hard_names.clone();
    for name in &soft_names {
        push_unique(&mut risky_tool_names, name);
    }

    ToolRiskAssessment {
        risky_tool_hard: !hard_names.is_empty(),
        risky_tool_soft: !soft_names.is_empty(),
        risky_tool_names,
        risky_tool_hard_names: hard_names,
        risky_tool_soft_names: soft_names,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolTier {
    None,
    Hard,
    Soft,
}

fn classify_tool_call(name: &str, arguments: &str) -> ToolTier {
    if is_delete_tool_name(name) {
        return ToolTier::Hard;
    }
    if soft_canonical_name(name).is_some() {
        return ToolTier::Soft;
    }
    if matches!(canonical_mutable_name(name), Some("exec")) && exec_is_delete_or_move(arguments) {
        return ToolTier::Hard;
    }
    ToolTier::None
}

/// Human-readable label for hard-tier reason codes.
fn hard_label(name: &str, arguments: &str) -> String {
    if is_delete_tool_name(name) {
        return "delete".into();
    }
    if exec_is_delete_or_move(arguments) {
        return "exec".into();
    }
    canonical_mutable_name(name)
        .unwrap_or(name)
        .to_string()
}

fn is_delete_tool_name(name: &str) -> bool {
    matches!(name, "Delete" | "delete")
}

fn soft_canonical_name(name: &str) -> Option<&'static str> {
    match name {
        "browser" => Some("browser"),
        "sessions_spawn" => Some("sessions_spawn"),
        "message" => Some("message"),
        _ => None,
    }
}

fn canonical_mutable_name(name: &str) -> Option<&'static str> {
    match name {
        "Shell" | "shell" | "run_terminal_cmd" | "exec" => Some("exec"),
        "Write" | "write" | "StrReplace" | "search_replace" | "edit" => None,
        _ => None,
    }
}

fn push_unique(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|existing| existing == name) {
        list.push(name.to_string());
    }
}

fn exec_is_delete_or_move(arguments: &str) -> bool {
    let Some(cmd) = extract_exec_command(arguments) else {
        return false;
    };
    let lower = cmd.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    exec_command_deletes(&lower) || exec_command_moves(&lower)
}

fn extract_exec_command(arguments: &str) -> Option<String> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(s) = v.get("command").and_then(|c| c.as_str()) {
            return Some(s.to_string());
        }
    }
    Some(trimmed.to_string())
}

fn exec_command_deletes(cmd: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "rm ",
        "rm\t",
        "rmdir ",
        "del ",
        "erase ",
        "unlink ",
        "shred ",
        "git clean",
        "git rm",
        "remove-item ",
        "remove-item\t",
        "trash ",
    ];
    PREFIXES.iter().any(|p| cmd.starts_with(p))
        || cmd == "rm"
        || cmd.contains(" remove-item ")
        || cmd.starts_with("powershell -command remove")
        || cmd.starts_with("powershell remove")
}

fn exec_command_moves(cmd: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "mv ",
        "mv\t",
        "move ",
        "ren ",
        "rename ",
        "git mv",
        "move-item ",
        "move-item\t",
    ];
    PREFIXES.iter().any(|p| cmd.starts_with(p))
        || cmd == "mv"
        || cmd.contains(" move-item ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::api::openai::{FunctionCallPayload, ToolCall};

    fn call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            call_type: "function".into(),
            function: FunctionCallPayload {
                name: name.into(),
                arguments: args.into(),
            },
        }
    }

    #[test]
    fn exec_rm_is_hard() {
        let a = assess_tool_calls(&[call("exec", r#"{"command":"rm -rf /tmp/x"}"#)]);
        assert!(a.risky_tool_hard);
        assert_eq!(a.risky_tool_hard_names, vec!["exec"]);
        assert!(!a.risky_tool_soft);
    }

    #[test]
    fn exec_mv_is_hard() {
        let a = assess_tool_calls(&[call("exec", r#"{"command":"mv a b"}"#)]);
        assert!(a.risky_tool_hard);
    }

    #[test]
    fn exec_git_status_is_none() {
        let a = assess_tool_calls(&[call("exec", r#"{"command":"git status"}"#)]);
        assert!(!a.risky_tool_hard);
        assert!(!a.risky_tool_soft);
    }

    #[test]
    fn write_create_is_none() {
        let a = assess_tool_calls(&[call("write", r#"{"path":"a.txt","content":"hi"}"#)]);
        assert!(!a.risky_tool_hard);
        assert!(!a.risky_tool_soft);
    }

    #[test]
    fn delete_tool_is_hard() {
        let a = assess_tool_calls(&[call("Delete", r#"{"path":"a.txt"}"#)]);
        assert!(a.risky_tool_hard);
        assert_eq!(a.risky_tool_hard_names, vec!["delete"]);
    }

    #[test]
    fn browser_is_soft() {
        let a = assess_tool_calls(&[call("browser", r#"{"query":"gold price"}"#)]);
        assert!(a.risky_tool_soft);
        assert!(!a.risky_tool_hard);
    }

    #[test]
    fn edit_is_none() {
        let a = assess_tool_calls(&[call("edit", r#"{"path":"a.txt"}"#)]);
        assert!(!a.risky_tool_hard);
        assert!(!a.risky_tool_soft);
    }

    #[test]
    fn read_is_none() {
        let a = assess_tool_calls(&[call("read", r#"{"path":"a.txt"}"#)]);
        assert!(!a.risky_tool_hard);
        assert!(!a.risky_tool_soft);
    }
}
