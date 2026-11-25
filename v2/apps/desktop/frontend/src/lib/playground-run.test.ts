/**
 * playground-run 单测（product-design §9.1 / Phase D1）：
 *
 * - 逐列独立完成：deferred 控制 resolveA，断言 B 仍 pending，再 resolveB；
 * - "停止等待"= 前端放弃等待（abandonColumns），迟到结果被 runId 守卫丢弃；
 * - `ok: false` 响应不是成功；
 * - runId 守卫用生产同一个 acceptColumnUpdate + mergeColumn；
 * - TPS 口径与对比结论（≥2 成功列才有结论）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { PlaygroundExecuteResponse } from '../api/types'
import {
  abandonColumns,
  acceptColumnUpdate,
  columnOutcome,
  compareConclusion,
  mergeColumn,
  runTargets,
  tpsOf,
  type RunColumn,
} from './playground-run'

function response(overrides: Partial<PlaygroundExecuteResponse> = {}): PlaygroundExecuteResponse {
  return {
    ok: true,
    served_model: 'test',
    text: 'hello',
    reasoning: '',
    ttft_ms: 100,
    duration_ms: 1000,
    input_tokens: 10,
    output_tokens: 20,
    finish_reason: 'stop',
    error: null,
    ...overrides,
  }
}

function makeColumn(overrides: Partial<RunColumn> = {}): RunColumn {
  return {
    target: 'a',
    label: 'A',
    running: false,
    abandoned: false,
    slow: false,
    result: null,
    error: null,
    ...overrides,
  }
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

test('先完成的列先回调：resolveA 后 B 仍在 running（逐列独立更新）', async () => {
  const updates: Array<{ runId: number; column: RunColumn }> = []
  const a = deferred<PlaygroundExecuteResponse>()
  const b = deferred<PlaygroundExecuteResponse>()
  const promises = runTargets({
    targets: [
      { target: 'candidate:a', label: 'A' },
      { target: 'candidate:b', label: 'B' },
    ],
    body: { messages: [{ role: 'user', content: 'hi' }], max_tokens: 8 },
    runId: 1,
    execute: (target) => (target === 'candidate:a' ? a.promise : b.promise),
    onColumn: (runId, column) => updates.push({ runId, column }),
  })
  void promises
  // A 完成，B 未 resolve。
  a.resolve(response({ served_model: 'a' }))
  await a.promise
  await Promise.resolve()
  await Promise.resolve()
  const bColumn = updates.find((update) => update.column.target === 'candidate:b')!
  assert.equal(bColumn.column.running, true)
  const aDone = updates.find(
    (update) => update.column.target === 'candidate:a' && !update.column.running,
  )
  assert.ok(aDone, 'A 的完成回调先于 B 到达')
  // B 还没有任何非 running 回调：A 完成时 B 确实还在等待。
  const bDoneYet = updates.find(
    (update) => update.column.target === 'candidate:b' && !update.column.running,
  )
  assert.equal(bDoneYet, undefined)
  // B 完成后才有 B 的非 running 回调。
  b.resolve(response({ served_model: 'b' }))
  await Promise.all(promises)
  const bDone = updates.find(
    (update) => update.column.target === 'candidate:b' && !update.column.running,
  )
  assert.ok(bDone, 'B 最终完成')
})

test('ok=false 的响应不是成功（不能显示完成/绿标）', () => {
  const column = makeColumn({
    target: 'virtual:smart',
    result: response({ ok: false, error: '上游 401', text: '' }),
  })
  assert.equal(columnOutcome(column), 'failed')
})

test('result.error 非空同样判失败', () => {
  const column = makeColumn({ result: response({ error: 'overloaded' }) })
  assert.equal(columnOutcome(column), 'failed')
})

test('停止等待：abandonColumns 标记放弃列；迟到结果被 runId 守卫丢弃', () => {
  // UI 侧：停止等待时 runId 前进 + abandonColumns。
  const columns = [makeColumn({ running: true })]
  const afterStop = abandonColumns(columns)
  assert.equal(afterStop[0].abandoned, true)
  assert.equal(columnOutcome(afterStop[0]), 'abandoned')

  // 旧轮（runId=1）迟到结果被守卫拒绝；新一轮（runId=2）结果被接受。
  const lateUpdate = makeColumn({ target: 'a', result: response({ served_model: 'late' }) })
  assert.equal(acceptColumnUpdate(1, 2), false, '旧轮回调不进新一轮列')
  assert.equal(acceptColumnUpdate(2, 2), true)
  // UI 流程：守卫拒绝 → 不调用 mergeColumn，afterStop 原样保留"已停止等待"；
  // 守卫放行时 mergeColumn 才替换列（这里验证替换语义本身）。
  assert.equal(afterStop[0].abandoned, true)
  const replaced = mergeColumn(afterStop, lateUpdate)
  assert.equal(replaced[0].result?.served_model, 'late')
})

test('runTargets 迟到回调在重复 run 下不串结果（生产守卫复用）', async () => {
  const updates: Array<{ runId: number; column: RunColumn }> = []
  const gate = deferred<PlaygroundExecuteResponse>()
  const promises = runTargets({
    targets: [{ target: 'candidate:a', label: 'A' }],
    body: { messages: [{ role: 'user', content: 'hi' }], max_tokens: 8 },
    runId: 1,
    execute: () => gate.promise,
    onColumn: (runId, column) => updates.push({ runId, column }),
  })
  // 新一轮开始：当前轮 runId 变为 2，旧轮列被重建。
  let currentRunId = 2
  const rebuiltColumns: RunColumn[] = []
  gate.resolve(response({ served_model: 'old' }))
  await Promise.all(promises)
  for (const update of updates) {
    if (!acceptColumnUpdate(update.runId, currentRunId)) continue
    const merged = mergeColumn(rebuiltColumns, update.column)
    if (merged !== rebuiltColumns) rebuiltColumns.push(update.column)
  }
  // 旧轮回调全部被守卫拒绝，新一轮列未被旧数据污染。
  assert.equal(rebuiltColumns.length, 0)
})

test('abandonColumns 只影响运行中的列', () => {
  const running = makeColumn({ running: true })
  const done = makeColumn({ result: response({}) })
  const [a, b] = abandonColumns([running, done])
  assert.equal(a.abandoned, true)
  assert.equal(a.running, false)
  assert.equal(b.abandoned, false)
  assert.equal(columnOutcome(b), 'succeeded')
})

test('TPS 口径：输出 token / 总耗时；无输出或无耗时时为 null', () => {
  assert.equal(tpsOf(response({ output_tokens: 20, duration_ms: 1000 })), 20)
  assert.equal(tpsOf(response({ output_tokens: 0, duration_ms: 1000 })), null)
  assert.equal(tpsOf(response({ output_tokens: 5, duration_ms: 0 })), null)
})

test('对比结论：≥2 成功列才有结论；单列或全失败为 null', () => {
  const fast = makeColumn({
    target: 'candidate:fast',
    label: 'fast',
    result: response({ ttft_ms: 90, duration_ms: 500 }),
  })
  const slow = makeColumn({
    target: 'candidate:slow',
    label: 'slow',
    result: response({ ttft_ms: 400, duration_ms: 2000 }),
  })
  const failed = makeColumn({ target: 'candidate:bad', label: 'bad', error: 'boom' })
  const conclusion = compareConclusion([fast, slow])
  assert.equal(conclusion.fastestFirstEvent, 'fast')
  assert.equal(conclusion.shortestTotalMs, 'fast')
  const conclusionSkipFailed = compareConclusion([fast, slow, failed])
  assert.equal(conclusionSkipFailed.shortestTotalMs, 'fast')
  assert.equal(compareConclusion([fast]).shortestTotalMs, null)
  const none = compareConclusion([failed])
  assert.equal(none.fastestFirstEvent, null)
  assert.equal(none.shortestTotalMs, null)
})
