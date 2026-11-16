import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { formatAxisTick } from './format-number'

describe('formatAxisTick', () => {
  it('keeps distinct fractional ticks when scale is below 1k', () => {
    assert.equal(formatAxisTick(0), '0')
    assert.equal(formatAxisTick(0.25), '0.25')
    assert.equal(formatAxisTick(0.5), '0.5')
    assert.equal(formatAxisTick(0.972), '0.97')
    assert.equal(formatAxisTick(1), '1')
  })

  it('uses compact notation for large values', () => {
    assert.equal(formatAxisTick(1500), '1.5K')
    assert.equal(formatAxisTick(2_500_000), '2.5M')
  })
})
