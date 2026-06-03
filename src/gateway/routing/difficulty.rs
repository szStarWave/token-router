use super::signals::RequestSignals;
use super::step_kind::StepKind;

#[derive(Debug, Clone, Copy)]
pub struct DifficultyScore(pub f32);

impl DifficultyScore {
    pub fn compute(
        signals: &RequestSignals,
        step_kind: StepKind,
        ctx_edge_max: u32,
        experience_bias: f32,
    ) -> Self {
        let ctx_ratio = (signals.tok_loop_delta as f32) / ctx_edge_max as f32;
        let tool_ratio = (signals.n_tool_defs as f32) / 20.0;
        let code_hint = 0.0f32;

        let mut linear = 0.40 * ctx_ratio.min(1.0)
            + 0.15 * tool_ratio.min(1.0)
            + 0.25 * if signals.intent_hard { 1.0 } else { 0.0 }
            - 0.35 * if signals.intent_easy { 1.0 } else { 0.0 }
            + 0.10 * if signals.multimodal { 1.0 } else { 0.0 }
            + code_hint
            + step_kind.bias()
            + experience_bias;

        if signals.assistant_failed_recent {
            linear += 0.15;
        }

        let d = sigmoid(linear);
        Self(d.clamp(0.0, 1.0))
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}
