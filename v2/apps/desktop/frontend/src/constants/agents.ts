/** 桌面端有精确合并器的 8 个 agent（Onboarding 与智能体工作台共用）。 */
export const AGENTS = [
  { id: 'openclaw', label: 'OpenClaw', icon: 'agent-sparkle' },
  { id: 'hermes', label: 'Hermes', icon: 'agent-bot' },
  { id: 'hermes-flash', label: 'Hermes Flash', icon: 'agent-sparkle' },
  { id: 'claude-code', label: 'Claude Code', icon: 'agent-terminal' },
  { id: 'codex', label: 'Codex', icon: 'agent-terminal' },
  { id: 'opencode', label: 'OpenCode', icon: 'agent-code' },
  { id: 'codebuddy', label: 'CodeBuddy', icon: 'agent-code' },
  { id: 'workbuddy', label: 'WorkBuddy', icon: 'agent-code' },
] as const

export type AgentId = (typeof AGENTS)[number]['id']

/**
 * 更多 agent（手动接入）：精确合并器只覆盖上面 8 个；
 * 其余用统一环境变量接入（base URL 指到网关，key 用 inference key）。
 */
export const MANUAL_AGENTS = [
  { id: 'cursor', label: 'Cursor（CLI / IDE）', env: 'OPENAI_BASE_URL' },
  { id: 'cline', label: 'Cline / Roo Code', env: 'OPENAI_BASE_URL' },
  { id: 'continue', label: 'Continue', env: 'OPENAI_BASE_URL' },
  { id: 'zed', label: 'Zed', env: 'OPENAI_BASE_URL' },
  { id: 'goose', label: 'Goose', env: 'OPENAI_BASE_URL' },
  { id: 'crush', label: 'Crush', env: 'OPENAI_BASE_URL' },
  { id: 'aider', label: 'Aider', env: 'OPENAI_API_BASE' },
  { id: 'gemini-cli', label: 'Gemini CLI', env: 'GOOGLE_GEMINI_BASE_URL' },
  { id: 'qwen-code', label: 'Qwen Code', env: 'OPENAI_BASE_URL' },
  { id: 'kimi-code', label: 'Kimi Code', env: 'OPENAI_BASE_URL' },
  { id: 'copilot-cli', label: 'Copilot CLI', env: 'OPENAI_BASE_URL' },
  { id: 'cherry-studio', label: 'Cherry Studio', env: 'OPENAI_BASE_URL' },
  { id: 'chatbox', label: 'Chatbox', env: 'OPENAI_BASE_URL' },
  { id: 'droid', label: 'Droid', env: 'OPENAI_BASE_URL' },
] as const
