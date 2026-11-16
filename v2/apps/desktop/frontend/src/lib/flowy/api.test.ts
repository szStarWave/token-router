import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { parseCloudModelContextWindow } from './api'

describe('parseCloudModelContextWindow', () => {
  it('parses context_window from extra JSON string', () => {
    const extra = '{"input": ["text"], "reasoning": true, "max_tokens": 384000, "credit_rate": 1.8, "context_window": 1000000}'
    assert.equal(parseCloudModelContextWindow(extra), 1_000_000)
  })

  it('parses context_window from extra object', () => {
    assert.equal(parseCloudModelContextWindow({ context_window: 262144 }), 262_144)
  })

  it('returns undefined for invalid extra', () => {
    assert.equal(parseCloudModelContextWindow('not-json'), undefined)
    assert.equal(parseCloudModelContextWindow(null), undefined)
    assert.equal(parseCloudModelContextWindow({}), undefined)
  })
})
