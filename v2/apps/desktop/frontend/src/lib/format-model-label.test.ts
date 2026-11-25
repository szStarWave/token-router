import assert from 'node:assert/strict'
import { test } from 'node:test'
import { formatModelLabel } from '../pages/activity/shared'

test('formatModelLabel full 原样返回组/成员 id', () => {
  assert.equal(formatModelLabel('auto/Qwen3.5-9B', 'full'), 'auto/Qwen3.5-9B')
})

test('formatModelLabel short 只去掉路由组段', () => {
  assert.equal(formatModelLabel('auto/Qwen3.5-9B', 'short'), 'Qwen3.5-9B')
})

test('formatModelLabel short 保留成员自带的 /（上游 model 可能是 org/name）', () => {
  assert.equal(
    formatModelLabel('auto/meta-llama/Llama-3.1-8B', 'short'),
    'meta-llama/Llama-3.1-8B',
  )
})

test('formatModelLabel 没有 / 时原样返回', () => {
  assert.equal(formatModelLabel('auto', 'short'), 'auto')
})
