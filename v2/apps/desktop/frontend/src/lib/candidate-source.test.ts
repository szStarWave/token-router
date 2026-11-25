import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { ProviderInfo } from '../api/types'
import { providerOfCandidate, sanitizeCandidatePart } from './candidate-source'

const provider = (id: string, displayName = id) => ({ id, display_name: displayName }) as ProviderInfo

test('sanitizeCandidatePart 与服务端 sanitize 一致', () => {
  assert.equal(sanitizeCandidatePart('siliconflow/abc:1'), 'siliconflow-abc-1')
  assert.equal(sanitizeCandidatePart('a_b.c-d'), 'a_b.c-d')
})

test('providerOfCandidate 按编译 id 前缀还原供应商', () => {
  const providers = [provider('siliconflow-bccdda58', '硅基流动')]
  assert.equal(
    providerOfCandidate(providers, 'siliconflow-bccdda58-BAAI-bge-1c72fadb')?.id,
    'siliconflow-bccdda58',
  )
})

test('providerOfCandidate 优先最长前缀', () => {
  const providers = [provider('foo'), provider('foo-bar')]
  assert.equal(providerOfCandidate(providers, 'foo-bar-model-1234')?.id, 'foo-bar')
  assert.equal(providerOfCandidate(providers, 'foo-model-1234')?.id, 'foo')
})

test('providerOfCandidate 对匹配不到的手写候选返回 undefined', () => {
  const providers = [provider('siliconflow')]
  assert.equal(providerOfCandidate(providers, 'local-fast'), undefined)
  assert.equal(providerOfCandidate(providers, 'siliconflowish-model-1234'), undefined)
})
