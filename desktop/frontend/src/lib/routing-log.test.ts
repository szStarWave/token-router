import assert from 'node:assert/strict'
import test from 'node:test'
import {
  isRoutingLogLine,
  parseRoutingLogLine,
  pickDisplayReasonCodes,
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

test('picks decisive reason tags', () => {
  const { shown } = pickDisplayReasonCodes(['STEP_DIRECT_CHAT', 'GATE_CTX_OVERFLOW', 'DIFFICULTY_0.90'])
  assert.ok(shown.includes('GATE_CTX_OVERFLOW'))
  assert.ok(!shown.includes('STEP_DIRECT_CHAT'))
})
