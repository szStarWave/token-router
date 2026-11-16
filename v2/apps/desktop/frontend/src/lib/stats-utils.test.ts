import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { modelStatsKey, parseModelStatsKey, tierTokenSummary, tierTokenTotal, tokenSummary, formatReqPerMin } from './stats-utils'

describe('tierTokenTotal', () => {
  it('sums input and output only, excluding cached', () => {
    const tier = { input: 100, output: 50, cached: 20 }
    assert.equal(tierTokenTotal(tier), 150)
    assert.equal(tierTokenSummary(tier).total, 150)
  })
})

describe('tokenSummary vs tier totals', () => {
  it('matches cloud tier when all usage is cloud', () => {
    const tb = {
      edge: { input: 0, output: 0, cached: 0 },
      cloud: { input: 1000, output: 200, cached: 80 },
      total: { input: 1000, output: 200, cached: 80 },
    }
    assert.equal(tokenSummary(tb).total, 1200)
    assert.equal(tierTokenSummary(tb.cloud).total, 1200)
  })
})

describe('modelStatsKey', () => {
  it('roundtrips tier and model', () => {
    const key = modelStatsKey('cloud', 'gpt-4o')
    assert.equal(key, 'cloud:gpt-4o')
    assert.deepEqual(parseModelStatsKey(key), { tier: 'cloud', model: 'gpt-4o' })
  })
})

describe('formatReqPerMin', () => {
  it('keeps one decimal for small positive rates', () => {
    assert.equal(formatReqPerMin(0.34), '0.3')
    assert.equal(formatReqPerMin(1.2), '1.2')
    assert.equal(formatReqPerMin(12.6), '13')
  })
})
