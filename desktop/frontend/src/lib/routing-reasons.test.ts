import assert from 'node:assert/strict'
import test from 'node:test'
import { t } from '../i18n/dict.js'
import { explainReasonCode } from './routing-reasons.js'

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
