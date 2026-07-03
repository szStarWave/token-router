type TFn = (key: string, vars?: Record<string, string | number>) => string

const EXACT_KEYS: Record<string, string> = {
  GATE_EDGE_DOWN: 'routing.reason.GATE_EDGE_DOWN',
  GATE_USER_REJECT: 'routing.reason.GATE_USER_REJECT',
  GATE_CTX_OVERFLOW: 'routing.reason.GATE_CTX_OVERFLOW',
  GATE_ASSISTANT_FAILURE: 'routing.reason.GATE_ASSISTANT_FAILURE',
  GATE_TOOL_ERROR_STREAK: 'routing.reason.GATE_TOOL_ERROR_STREAK',
  GATE_RISKY_TOOL: 'routing.reason.GATE_RISKY_TOOL',
  GATE_OPENCLAW_COMPACT: 'routing.reason.GATE_OPENCLAW_COMPACT',
  GATE_EDGE_BUSY: 'routing.reason.GATE_EDGE_BUSY',
  UPSTREAM_EDGE_ONLY: 'routing.reason.UPSTREAM_EDGE_ONLY',
  UPSTREAM_CLOUD_ONLY: 'routing.reason.UPSTREAM_CLOUD_ONLY',
  PLAN_INTENT_CLOUD: 'routing.reason.PLAN_INTENT_CLOUD',
  INITIAL_PLAN_CLOUD: 'routing.reason.INITIAL_PLAN_CLOUD',
  WORK_EXEC_EDGE: 'routing.reason.WORK_EXEC_EDGE',
  WORK_CACHE_EDGE: 'routing.reason.WORK_CACHE_EDGE',
  MULTIMODAL_COMPLEX_CLOUD: 'routing.reason.MULTIMODAL_COMPLEX_CLOUD',
  MULTIMODAL_CACHE_EDGE: 'routing.reason.MULTIMODAL_CACHE_EDGE',
  MULTIMODAL_CACHE_CLOUD: 'routing.reason.MULTIMODAL_CACHE_CLOUD',
  MULTIMODAL_SIMPLE_EDGE: 'routing.reason.MULTIMODAL_SIMPLE_EDGE',
  MULTIMODAL_PROBE_EDGE: 'routing.reason.MULTIMODAL_PROBE_EDGE',
  STICKY_CASCADE_RETRY: 'routing.reason.STICKY_CASCADE_RETRY',
  CASUAL_PREFER_EDGE: 'routing.reason.CASUAL_PREFER_EDGE',
  CASUAL_EDGE_FALLBACK: 'routing.reason.CASUAL_EDGE_FALLBACK',
  LEXICAL_RARE: 'routing.reason.LEXICAL_RARE',
  LEXICAL_SPECIAL: 'routing.reason.LEXICAL_SPECIAL',
  LEXICAL_BOTH: 'routing.reason.LEXICAL_BOTH',
  BAYES_COLD_START: 'routing.reason.BAYES_COLD_START',
  CONFIG_ROUTE_edge: 'routing.reason.CONFIG_ROUTE_edge',
  CONFIG_ROUTE_cloud: 'routing.reason.CONFIG_ROUTE_cloud',
  CONFIG_ROUTE_cascade: 'routing.reason.CONFIG_ROUTE_cascade',
  STEP_HEARTBEAT_ACK: 'routing.reason.STEP_HEARTBEAT_ACK',
  STEP_DIRECT_CHAT: 'routing.reason.STEP_DIRECT_CHAT',
  STEP_RECOVERY_AFTER_FAILURE: 'routing.reason.STEP_RECOVERY_AFTER_FAILURE',
  STEP_TOOL_SELECT: 'routing.reason.STEP_TOOL_SELECT',
  STEP_TOOL_ARG_FILL: 'routing.reason.STEP_TOOL_ARG_FILL',
  STEP_TOOL_RESULT_DIGEST: 'routing.reason.STEP_TOOL_RESULT_DIGEST',
  STEP_INITIAL_PLAN: 'routing.reason.STEP_INITIAL_PLAN',
  STEP_FINAL_REPLY: 'routing.reason.STEP_FINAL_REPLY',
  STEP_SUBAGENT_SPAWN: 'routing.reason.STEP_SUBAGENT_SPAWN',
  STEP_MEMORY_COMPACT: 'routing.reason.STEP_MEMORY_COMPACT',
  STEP_CRON_BACKGROUND: 'routing.reason.STEP_CRON_BACKGROUND',
}

function explainDynamicCode(code: string, t: TFn): string | null {
  if (code.startsWith('DIFFICULTY_')) {
    const score = code.slice('DIFFICULTY_'.length)
    return t('routing.reason.DIFFICULTY', { score })
  }
  if (code.startsWith('TOK_IN_')) {
    return t('routing.reason.TOK_IN', { n: code.slice('TOK_IN_'.length) })
  }
  if (code.startsWith('TOK_DELTA_')) {
    return t('routing.reason.TOK_DELTA', { n: code.slice('TOK_DELTA_'.length) })
  }
  if (code.startsWith('EXP_BIAS_')) {
    return t('routing.reason.EXP_BIAS', { bias: code.slice('EXP_BIAS_'.length) })
  }
  if (code.startsWith('TOOL_ERROR_STREAK_')) {
    return t('routing.reason.TOOL_ERROR_STREAK', { n: code.slice('TOOL_ERROR_STREAK_'.length) })
  }
  if (code.startsWith('TOOL_LOOP_')) {
    return t('routing.reason.TOOL_LOOP', { n: code.slice('TOOL_LOOP_'.length) })
  }
  if (code.startsWith('BAYES_P(') && code.endsWith(')')) {
    return t('routing.reason.BAYES_P', { p: code.slice('BAYES_P('.length, -1) })
  }
  if (code.startsWith('ADAPTIVE_VERIFY(p=') && code.endsWith(')')) {
    return t('routing.reason.ADAPTIVE_VERIFY', { p: code.slice('ADAPTIVE_VERIFY(p='.length, -1) })
  }
  if (code.startsWith('ADAPTIVE_THETA(') && code.endsWith(')')) {
    const inner = code.slice('ADAPTIVE_THETA('.length, -1)
    const [edge, cloud] = inner.split(',')
    return t('routing.reason.ADAPTIVE_THETA', { edge: edge ?? '', cloud: cloud ?? '' })
  }
  if (code.startsWith('WORK_VERIFY_SAMPLE(p=') && code.endsWith(')')) {
    return t('routing.reason.WORK_VERIFY_SAMPLE', { p: code.slice('WORK_VERIFY_SAMPLE(p='.length, -1) })
  }
  if (code.startsWith('WORK_SAMPLE_SKIP(p=') && code.endsWith(')')) {
    return t('routing.reason.WORK_SAMPLE_SKIP', { p: code.slice('WORK_SAMPLE_SKIP(p='.length, -1) })
  }
  if (code.startsWith('CONFIG_ROUTE_')) {
    const tier = code.slice('CONFIG_ROUTE_'.length).toLowerCase()
    const key = `routing.reason.CONFIG_ROUTE_${tier}`
    const msg = t(key)
    if (msg !== key) return msg
  }
  if (code.startsWith('ADAPTIVE_')) {
    return t('routing.reason.ADAPTIVE_GENERIC', { code })
  }
  return null
}

export function explainReasonCode(code: string, t: TFn): string {
  const exactKey = EXACT_KEYS[code]
  if (exactKey) {
    const msg = t(exactKey)
    if (msg !== exactKey) return msg
  }
  const dynamic = explainDynamicCode(code, t)
  if (dynamic) return dynamic
  return t('routing.reason.UNKNOWN', { code })
}

export function stepKindLabel(stepKind: string, t: TFn): string {
  const key = `routing.stepKind.${stepKind}`
  const msg = t(key)
  return msg !== key ? msg : stepKind
}
