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
  PLAN_INTENT: 'routing.reason.PLAN_INTENT',
  PLAN_INTENT_CLOUD: 'routing.reason.PLAN_INTENT_CLOUD',
  CLOUD_INTENT: 'routing.reason.CLOUD_INTENT',
  LONG_GEN_EDGE: 'routing.reason.LONG_GEN_EDGE',
  LONG_GEN_EDGE_TPS_LOW: 'routing.reason.LONG_GEN_EDGE_TPS_LOW',
  LONG_GEN_CLOUD_ONLY: 'routing.reason.LONG_GEN_CLOUD_ONLY',
  EDGE_INTENT: 'routing.reason.EDGE_INTENT',
  INITIAL_PLAN: 'routing.reason.INITIAL_PLAN',
  INITIAL_PLAN_CLOUD: 'routing.reason.INITIAL_PLAN_CLOUD',
  ANALYSIS_INTENT: 'routing.reason.ANALYSIS_INTENT',
  DECISION_INTENT: 'routing.reason.DECISION_INTENT',
  RESEARCH_INTENT: 'routing.reason.RESEARCH_INTENT',
  WORK_EXEC_EDGE: 'routing.reason.WORK_EXEC_EDGE',
  WORK_TOOL_ERROR_VERIFY: 'routing.reason.WORK_TOOL_ERROR_VERIFY',
  WORK_CACHE_EDGE: 'routing.reason.WORK_CACHE_EDGE',
  MULTIMODAL_COMPLEX_CLOUD: 'routing.reason.MULTIMODAL_COMPLEX_CLOUD',
  MULTIMODAL_CACHE_EDGE: 'routing.reason.MULTIMODAL_CACHE_EDGE',
  MULTIMODAL_CACHE_CLOUD: 'routing.reason.MULTIMODAL_CACHE_CLOUD',
  MULTIMODAL_SIMPLE_EDGE: 'routing.reason.MULTIMODAL_SIMPLE_EDGE',
  MULTIMODAL_PROBE_EDGE: 'routing.reason.MULTIMODAL_PROBE_EDGE',
  CLOUD_CACHE_BOOST: 'routing.reason.CLOUD_CACHE_BOOST',
  REQ_ROUTE_CACHE_CLOUD: 'routing.reason.REQ_ROUTE_CACHE_CLOUD',
  REQ_ROUTE_CACHE_EDGE: 'routing.reason.REQ_ROUTE_CACHE_EDGE',
  STICKY_CASCADE_RETRY: 'routing.reason.STICKY_CASCADE_RETRY',
  CASUAL_PREFER_EDGE: 'routing.reason.CASUAL_PREFER_EDGE',
  CASUAL_EDGE_FALLBACK: 'routing.reason.CASUAL_EDGE_FALLBACK',
  CASUAL_CLASSIFIER_GUARD: 'routing.reason.CASUAL_CLASSIFIER_GUARD',
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
  CTX_RATIO: 'routing.reason.CTX_RATIO',
  USER_CTX_RATIO: 'routing.reason.USER_CTX_RATIO',
  TOOL_RATIO: 'routing.reason.TOOL_RATIO',
  INTENT_HARD: 'routing.reason.INTENT_HARD',
  INTENT_CLOUD: 'routing.reason.INTENT_CLOUD',
  INTENT_LONG_GEN: 'routing.reason.INTENT_LONG_GEN',
  INTENT_EDGE: 'routing.reason.INTENT_EDGE',
  INTENT_EASY: 'routing.reason.INTENT_EASY',
  USER_MULTIMODAL: 'routing.reason.USER_MULTIMODAL',
  ASSISTANT_FAILED: 'routing.reason.ASSISTANT_FAILED',
  MEMORY_COMPACT_IN_BUDGET: 'routing.reason.MEMORY_COMPACT_IN_BUDGET',
  RISKY_TOOL_SOFT: 'routing.reason.RISKY_TOOL_SOFT',
}

export function explainDifficultyPartKey(key: string, t: TFn): string {
  const exactKey = EXACT_KEYS[key]
  if (exactKey) {
    const msg = t(exactKey)
    if (msg !== exactKey) return msg
  }
  if (key.startsWith('STEP_')) {
    const step = key.slice('STEP_'.length).toLowerCase()
    return stepKindLabel(step, t)
  }
  if (key.startsWith('TOOL_ERROR_STREAK_')) {
    return t('routing.reason.TOOL_ERROR_STREAK', { n: key.slice('TOOL_ERROR_STREAK_'.length) })
  }
  if (key.startsWith('TOOL_LOOP_')) {
    return t('routing.reason.TOOL_LOOP', { n: key.slice('TOOL_LOOP_'.length) })
  }
  if (key.startsWith('EXP_BIAS_')) {
    return t('routing.reason.EXP_BIAS', { bias: key.slice('EXP_BIAS_'.length) })
  }
  if (key.startsWith('CALIB_')) {
    const calibKey = `routing.reason.${key}`
    const msg = t(calibKey)
    if (msg !== calibKey) return msg
  }
  const calibAliases: Record<string, string> = {
    BAYES_FUSE: 'routing.reason.DIFF_D_BAYES_FUSE',
    PRIVACY_CAP: 'routing.reason.DIFF_D_PRIVACY_CAP',
    PRIVACY_RECOVERY_FLOOR: 'routing.reason.DIFF_D_PRIVACY_RECOVERY_FLOOR',
    CASUAL_CLASSIFIER_GUARD: 'routing.reason.CASUAL_CLASSIFIER_GUARD',
  }
  const alias = calibAliases[key]
  if (alias) {
    const msg = t(alias)
    if (msg !== alias) return msg
  }
  return t('routing.reason.UNKNOWN', { code: key })
}

function explainDynamicCode(code: string, t: TFn): string | null {
  if (code.startsWith('DIFFICULTY_')) {
    const score = code.slice('DIFFICULTY_'.length)
    return t('routing.reason.DIFFICULTY', { score })
  }
  if (code.startsWith('DIFF_L:')) {
    const rest = code.slice('DIFF_L:'.length)
    const sep = rest.lastIndexOf(':')
    if (sep > 0) {
      const key = rest.slice(0, sep)
      const linear = rest.slice(sep + 1)
      return t('routing.reason.DIFF_L', {
        factor: explainDifficultyPartKey(key, t),
        linear,
      })
    }
  }
  if (code.startsWith('DIFF_D:')) {
    const rest = code.slice('DIFF_D:'.length)
    const sep = rest.lastIndexOf(':')
    if (sep > 0) {
      const key = rest.slice(0, sep)
      const delta = rest.slice(sep + 1)
      return t('routing.reason.DIFF_D', {
        factor: explainDifficultyPartKey(key, t),
        delta,
      })
    }
  }
  if (code.startsWith('DIFF_HEUR:')) {
    return t('routing.reason.DIFF_HEUR', { score: code.slice('DIFF_HEUR:'.length) })
  }
  if (code.startsWith('DIFF_LINEAR_SUM:')) {
    return t('routing.reason.DIFF_LINEAR_SUM', { sum: code.slice('DIFF_LINEAR_SUM:'.length) })
  }
  if (code.startsWith('DIFF_FUSE:')) {
    const m = /^DIFF_FUSE:heur=([\d.]+)[|,]bayes=([\d.]+)[|,]w=([\d.]+)[|,]final=([\d.]+)$/.exec(code)
    if (m) {
      return t('routing.reason.DIFF_FUSE', {
        heur: m[1],
        bayes: m[2],
        w: m[3],
        final: m[4],
      })
    }
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
    const [edge, cloud] = inner.split(/[|,]/)
    return t('routing.reason.ADAPTIVE_THETA', { edge: edge ?? '', cloud: cloud ?? '' })
  }
  if (code.startsWith('WORK_VERIFY_SAMPLE(p=') && code.endsWith(')')) {
    return t('routing.reason.WORK_VERIFY_SAMPLE', { p: code.slice('WORK_VERIFY_SAMPLE(p='.length, -1) })
  }
  if (code.startsWith('WORK_SAMPLE_SKIP(p=') && code.endsWith(')')) {
    return t('routing.reason.WORK_SAMPLE_SKIP', { p: code.slice('WORK_SAMPLE_SKIP(p='.length, -1) })
  }
  if (code.startsWith('GATE_RISKY_TOOL:')) {
    return t('routing.reason.GATE_RISKY_TOOL_NAMED', {
      tools: code.slice('GATE_RISKY_TOOL:'.length),
    })
  }
  if (code.startsWith('RISKY_TOOL_SOFT:')) {
    return t('routing.reason.RISKY_TOOL_SOFT_NAMED', {
      tools: code.slice('RISKY_TOOL_SOFT:'.length),
    })
  }
  if (code.startsWith('REQ_ROUTE_CACHE_CLOUD:')) {
    return t('routing.reason.REQ_ROUTE_CACHE_CLOUD_MODEL', {
      model: code.slice('REQ_ROUTE_CACHE_CLOUD:'.length),
    })
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

export function explainFinalRouteFactor(
  code: string,
  route: 'edge' | 'cloud' | 'cascade',
  t: TFn,
): string {
  if (code.startsWith('DIFFICULTY_')) {
    const score = code.slice('DIFFICULTY_'.length)
    return t('routing.reason.DIFFICULTY_ROUTE', {
      score,
      route: t(`route.${route}`),
    })
  }
  return explainReasonCode(code, t)
}

export function stepKindLabel(stepKind: string, t: TFn): string {
  const key = `routing.stepKind.${stepKind}`
  const msg = t(key)
  return msg !== key ? msg : stepKind
}
