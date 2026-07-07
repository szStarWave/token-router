import assert from 'node:assert/strict'
import test from 'node:test'
import {
  effectiveRouteTier,
  extractDifficultyScore,
  formatTimeLabel,
  isDifficultyMetaCode,
  isOrphanReasonFragment,
  isRoutingLogLine,
  parseDifficultyBreakdown,
  parseRoutingLogLine,
  pickDisplayReasonCodes,
  pickFinalRouteFactorCode,
  truncatePreview,
} from './routing-log.js'

const SAMPLE =
  '2026-07-03T07:00:00.000000Z  INFO token_router::gateway::api::meta: routing: direct_chat → edge | policy DIFFICULTY_0.20 → edge agent_id="default" model="gpt-4o" route=edge step_kind=direct_chat reason_codes=STEP_DIRECT_CHAT,DIFFICULTY_0.20,TOK_IN_120 user_preview="请帮我总结会议"'

test('identifies routing lines', () => {
  assert.equal(isRoutingLogLine(SAMPLE), true)
  assert.equal(isRoutingLogLine('2026-07-03T07:00:00.000000Z  INFO x: served: edge'), false)
})

test('parses routing log line with user preview', () => {
  const entry = parseRoutingLogLine(SAMPLE, 1)
  assert.ok(entry)
  assert.equal(entry!.route, 'edge')
  assert.equal(entry!.stepKind, 'direct_chat')
  assert.equal(entry!.model, 'gpt-4o')
  assert.equal(entry!.userPreview, '请帮我总结会议')
  assert.deepEqual(entry!.reasonCodes, ['STEP_DIRECT_CHAT', 'DIFFICULTY_0.20', 'TOK_IN_120'])
})

test('truncates preview at 80 chars', () => {
  const long = 'a'.repeat(100)
  assert.equal(truncatePreview(long).length, 81)
})

test('formatTimeLabel uses local timezone for UTC iso', () => {
  const iso = '2026-07-03T07:00:00.000000Z'
  const expected = new Date(iso).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
  assert.equal(formatTimeLabel(iso), expected)
})

test('prefers served route over decision route', () => {
  assert.equal(effectiveRouteTier({ route: 'cloud', served_route: 'edge' }), 'edge')
})

test('picks decisive reason tags', () => {
  const { shown } = pickDisplayReasonCodes(['STEP_DIRECT_CHAT', 'GATE_CTX_OVERFLOW', 'DIFFICULTY_0.90'])
  assert.ok(shown.includes('GATE_CTX_OVERFLOW'))
  assert.ok(!shown.includes('STEP_DIRECT_CHAT'))
})

test('extracts difficulty from codes or fallback', () => {
  assert.equal(extractDifficultyScore(['DIFFICULTY_0.82']), 0.82)
  assert.equal(extractDifficultyScore(['STEP_DIRECT_CHAT'], 0.15), 0.15)
})

test('picks last route override as final factor', () => {
  const codes = [
    'STEP_TOOL_ARG_FILL',
    'TOOL_LOOP_9',
    'DIFFICULTY_0.00',
    'WORK_EXEC_EDGE',
    'WORK_CACHE_EDGE',
  ]
  assert.equal(pickFinalRouteFactorCode(codes), 'WORK_CACHE_EDGE')
})

test('falls back to difficulty when no override exists', () => {
  const codes = ['STEP_DIRECT_CHAT', 'DIFFICULTY_0.20', 'TOK_IN_120']
  assert.equal(pickFinalRouteFactorCode(codes), 'DIFFICULTY_0.20')
})

test('prefers difficulty over work execution tags', () => {
  const codes = [
    'STEP_TOOL_ARG_FILL',
    'DIFFICULTY_0.64',
    'WORK_EXEC_EDGE',
    'WORK_VERIFY_SAMPLE(p=0.10)',
  ]
  assert.equal(pickFinalRouteFactorCode(codes), 'DIFFICULTY_0.64')
})

test('prioritizes route overrides in display tags', () => {
  const { shown } = pickDisplayReasonCodes([
    'STEP_TOOL_ARG_FILL',
    'EXP_BIAS_-0.00',
    'TOOL_ERROR_STREAK_1',
    'TOOL_LOOP_215',
    'LEXICAL_RARE',
    'DIFFICULTY_0.07',
    'GATE_CTX_OVERFLOW',
  ])
  assert.equal(shown[0], 'GATE_CTX_OVERFLOW')
})

test('parses difficulty breakdown from reason codes', () => {
  const codes = [
    'DIFF_L:LEXICAL_RARE:+0.080',
    'DIFF_D:LEXICAL_RARE:+0.0123',
    'DIFF_LINEAR_SUM:-0.450',
    'DIFF_HEUR:0.3894',
    'DIFF_FUSE:heur=0.3894|bayes=0.0000|w=1.00|final=0.0000',
    'DIFF_D:BAYES_FUSE:-0.3894',
    'DIFFICULTY_0.00',
  ]
  assert.equal(isDifficultyMetaCode('DIFF_L:CTX_RATIO:+0.042'), true)
  const breakdown = parseDifficultyBreakdown(codes)
  assert.ok(breakdown)
  assert.equal(breakdown!.parts.length, 1)
  assert.equal(breakdown!.parts[0].key, 'LEXICAL_RARE')
  assert.equal(breakdown!.parts[0].linear, 0.08)
  assert.equal(breakdown!.heuristic, 0.3894)
  assert.equal(breakdown!.fuse?.final, 0)
  assert.equal(breakdown!.adjustments.length, 1)
  assert.equal(breakdown!.final, 0)
})

test('filters orphan fuse fragments from display', () => {
  assert.equal(isOrphanReasonFragment('bayes=0.6207'), true)
  assert.equal(isOrphanReasonFragment('w=1.00'), true)
  assert.equal(isOrphanReasonFragment('final=0.6207'), true)
  assert.equal(isOrphanReasonFragment('DIFFICULTY_0.62'), false)
})
