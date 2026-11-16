import assert from 'node:assert/strict'
import test from 'node:test'
import { t } from '../i18n/dict.js'
import { explainFinalRouteFactor, explainReasonCode } from './routing-reasons.js'

test('explains exact gate codes', () => {
  const msg = explainReasonCode('GATE_CTX_OVERFLOW', (key) => t('zh', key))
  assert.ok(msg.includes('80%'))
})

test('explains dynamic difficulty code', () => {
  const msg = explainReasonCode('DIFFICULTY_0.82', (key, vars) => t('zh', key, vars))
  assert.ok(msg.includes('0.82'))
})

test('falls back for unknown codes', () => {
  const msg = explainReasonCode('CUSTOM_FOO', (key, vars) => t('zh', key, vars))
  assert.ok(msg.includes('CUSTOM_FOO'))
})

test('explains final difficulty factor with route', () => {
  const msg = explainFinalRouteFactor('DIFFICULTY_0.20', 'edge', (key, vars) => t('zh', key, vars))
  assert.ok(msg.includes('0.20'))
  assert.ok(msg.includes('端侧'))
})

test('explains work verify sample as final factor', () => {
  const msg = explainFinalRouteFactor(
    'WORK_VERIFY_SAMPLE(p=0.10)',
    'cascade',
    (key, vars) => t('zh', key, vars),
  )
  assert.ok(msg.includes('0.10'))
  assert.ok(msg.includes('级联'))
})
