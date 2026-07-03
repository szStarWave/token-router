mod edge_busy;
mod adaptive;
mod adaptive_tuner;
mod conversation;
mod decision;
mod difficulty;
mod gates;
mod policy;
mod keywords;
mod lexical;
mod lexical_tokenizer;
mod wordfreq_seed;
mod wordfreq_store;
mod reject_intent;
mod signals;
mod step_kind;
mod tool_risk;
mod upstream_availability;
mod work;

pub use keywords::response_has_uncertainty;
pub use adaptive::{EffectiveRouting, compute_effective_routing};
pub use adaptive_tuner::AdaptiveTuner;
pub use conversation::conversation_key;

#[cfg(test)]
mod tests;

pub use decision::{RouteDecision, RouteTier, RoutingMode, decide};
pub use policy::Profile;
pub use difficulty::DifficultyScore;
pub use signals::{last_message_text, RequestSignals, SignalExtractor};
pub use step_kind::StepKind;
pub use upstream_availability::require_any_upstream;
pub use wordfreq_store::{LexicalLearnContext, WordFreqSettings, WordFreqStore};
pub use work::{WorkStrategy, apply_work_route, is_plan_step, is_work_step};
