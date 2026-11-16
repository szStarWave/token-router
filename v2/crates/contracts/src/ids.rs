//! 标识类型（§4.4、§4.9、§8.1）。
//!
//! 一律用 `String` 别名而不是 newtype：
//! - 这些 id 出现在配置、账本、响应头、SQLite 主键中，跨边界时不希望反复转换；
//! - 冲突风险由“命名空间”而非类型系统承担（语义见各文档）。
//!
//! 唯一例外是 `SlotRef`，因为它是结构体而非裸 id。

/// 物理候选 id，配置里的 `[[candidates]] id`。
pub type CandidateId = String;

/// 档位标签（`local` / `cloud` / `premium` / 自定义），**不是路由结论**。
pub type TierId = String;

/// 策略 id，注册表键。
pub type StrategyId = String;

/// 虚拟模型 id，客户端 `/v1/models` 看到的选择单位。
pub type VirtualModelId = String;

/// 上游物理模型 id（如 `claude-sonnet-4-5`），只出现在 Dispatch 与账本。
pub type PhysicalModelId = String;

/// profile id，阈值/权重/模式集合的名字。
pub type ProfileId = String;

/// 会话分支 id（fork / `/tree`）；默认 `"main"`。
pub type BranchId = String;

/// 会话键：64 位十六进制 SHA-256（§8.1）。
pub type SessionKey = String;

/// 可插拔信号 id（`SignalBag` 键）。
pub type SignalId = String;

/// 判官问题请求 id，用于 `PlanDraft.judge_requests` 与 verdict 关联。
pub type JudgeRequestId = String;

/// thinking level：`off | low | medium | high | custom`（按物理模型能力 clamp）。
pub type ThinkingLevel = String;

/// 任务 id：latch 作用域 `(session_key, task_id)` 的后半段；无任务时为空串。
pub type TaskId = String;
