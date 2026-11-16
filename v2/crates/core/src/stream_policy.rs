//! 流式切换边界（§10.6、P0b-10）。
//!
//! v2 的默认语义：
//! - `NextTurn` / `Off`：**不**在流式轮内换候选；升级只对下一轮生效。
//! - `GateBeforeTool`（P2）：只在 tool_call 前缓冲时可换，且必须尚未向客户端提交字节。
//!
//! 这个函数是唯一允许回答"能不能在流中途换"的地方；执行层不得自己判断。

use tr_contracts::StreamEscalation;

/// 已提交（或默认模式）时是否允许切换候选。
pub fn may_switch_after_commit(escalation: StreamEscalation, committed: bool) -> bool {
    match escalation {
        StreamEscalation::GateBeforeTool => !committed,
        StreamEscalation::NextTurn | StreamEscalation::Off => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_turn_never_switches_midstream() {
        assert!(!may_switch_after_commit(StreamEscalation::NextTurn, false));
        assert!(!may_switch_after_commit(StreamEscalation::NextTurn, true));
        assert!(!may_switch_after_commit(StreamEscalation::Off, false));
    }

    #[test]
    fn gate_before_tool_only_before_commit() {
        assert!(may_switch_after_commit(
            StreamEscalation::GateBeforeTool,
            false
        ));
        assert!(!may_switch_after_commit(
            StreamEscalation::GateBeforeTool,
            true
        ));
    }
}
