/**
 * config-draft-merge 单测：三方合并（自动保存 409 rebase 的核心算法）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mergeDraft } from './config-draft-merge'

test('只有一方改的字段各取其所（无冲突）', () => {
  const base = { server: { listen: 'a', auth: false }, defaultVirtualModel: 'auto' }
  const ours = { server: { listen: 'b', auth: false }, defaultVirtualModel: 'auto' }
  const theirs = { server: { listen: 'a', auth: true }, defaultVirtualModel: 'coder' }
  const { merged, conflicts } = mergeDraft(base, ours, theirs)
  assert.deepEqual(conflicts, [])
  assert.deepEqual(merged, {
    server: { listen: 'b', auth: true },
    defaultVirtualModel: 'coder',
  })
})

test('同一字段双方都改：冲突，先保留我的值并报告路径', () => {
  const base = { server: { listen: 'a' } }
  const ours = { server: { listen: 'b' } }
  const theirs = { server: { listen: 'c' } }
  const { merged, conflicts } = mergeDraft(base, ours, theirs)
  assert.deepEqual(conflicts, ['server.listen'])
  assert.deepEqual(merged, { server: { listen: 'b' } })
})

test('数组按 id 对齐：双方改不同元素可以合并', () => {
  const base = {
    candidates: [
      { id: 'a', model: 'm1', strength: 10 },
      { id: 'b', model: 'm2', strength: 20 },
    ],
  }
  const ours = {
    candidates: [
      { id: 'a', model: 'm1-new', strength: 10 },
      { id: 'b', model: 'm2', strength: 20 },
    ],
  }
  const theirs = {
    candidates: [
      { id: 'a', model: 'm1', strength: 10 },
      { id: 'b', model: 'm2', strength: 30 },
    ],
  }
  const { merged, conflicts } = mergeDraft(base, ours, theirs)
  assert.deepEqual(conflicts, [])
  assert.deepEqual(merged, {
    candidates: [
      { id: 'a', model: 'm1-new', strength: 10 },
      { id: 'b', model: 'm2', strength: 30 },
    ],
  })
})

test('数组同一元素同一字段：冲突路径带 id', () => {
  const base = { candidates: [{ id: 'a', model: 'm1' }] }
  const ours = { candidates: [{ id: 'a', model: 'mine' }] }
  const theirs = { candidates: [{ id: 'a', model: 'theirs' }] }
  const { merged, conflicts } = mergeDraft(base, ours, theirs)
  assert.deepEqual(conflicts, ['candidates[a].model'])
  assert.equal(merged.candidates[0].model, 'mine')
})

test('数组新增元素：双方各加一个都保留', () => {
  const base = { candidates: [{ id: 'a', model: 'm1' }] }
  const ours = { candidates: [{ id: 'a', model: 'm1' }, { id: 'mine', model: 'x' }] }
  const theirs = { candidates: [{ id: 'a', model: 'm1' }, { id: 'theirs', model: 'y' }] }
  const { merged, conflicts } = mergeDraft(base, ours, theirs)
  assert.deepEqual(conflicts, [])
  assert.deepEqual(
    merged.candidates.map((item) => item.id).sort(),
    ['a', 'mine', 'theirs'],
  )
})

test('删除 vs 修改同一元素：冲突；两边都删：删除生效', () => {
  const base = { candidates: [{ id: 'a', model: 'm1' }, { id: 'b', model: 'm2' }] }
  const ours = { candidates: [{ id: 'a', model: 'm1' }] } // 我删了 b
  const theirsDeleted = { candidates: [{ id: 'b', model: 'm2' }] } // 对方删了 a，b 未动
  const theirsModified = {
    candidates: [
      { id: 'a', model: 'm1' },
      { id: 'b', model: 'm2-changed' },
    ],
  }

  const merged1 = mergeDraft(base, ours, theirsDeleted)
  assert.deepEqual(merged1.conflicts, [], '我删 b、对方删 a：两边删除都生效')
  assert.deepEqual(merged1.merged.candidates, [])

  const merged2 = mergeDraft(base, ours, theirsModified)
  assert.deepEqual(merged2.conflicts, ['candidates[b]'], '我删 b、对方改 b：冲突')
})

test('非 id 数组按原子值处理：双方都改则冲突', () => {
  const base = { strategies: ['baseline'] }
  const ours = { strategies: ['baseline', 'fast'] }
  const theirs = { strategies: ['baseline', 'sticky'] }
  const { conflicts } = mergeDraft(base, ours, theirs)
  assert.deepEqual(conflicts, ['strategies'])
})
