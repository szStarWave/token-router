/**
 * Playground 运行编排（product-design §9.1）：与 UI 解耦的纯逻辑，便于单测。
 *
 * - 逐目标 await：先完成的列先更新，不 Promise.all 后才统一渲染；
 * - "停止等待"= 前端放弃等待，不是取消上游——上游请求可能继续并产生费用；
 *   机制只有一个：runId 守卫。停止等待时 runId 前进，迟到的旧轮回调被丢弃，
 *   列的"已停止等待"状态由 abandonColumns 一次性写入；
 * - `ok: false` 的响应不是成功（服务端聚合返回 HTTP 200 但执行失败）；
 * - TPS = 输出 token / 总耗时（含排队与网络，非纯生成速率）。
 */
import type { PlaygroundExecuteResponse } from '../api/types'
import { api } from '../api/client'

/** 一列结果（PlaygroundPage 的列状态）。 */
export interface RunColumn {
  target: string
  label: string
  /** 还没拿到结果。 */
  running: boolean
  /** 已停止等待（前端放弃，非取消上游）。 */
  abandoned: boolean
  /** 运行中超过 WAIT_HINT_MS。 */
  slow: boolean
  result: PlaygroundExecuteResponse | null
  error: string | null
}

/** 运行中"仍在等待完整结果"的提示阈值。 */
export const WAIT_HINT_MS = 30_000

/** 与 api.playgroundExecute 兼容的请求体（不含 target）。 */
export interface PlaygroundRunBody {
  messages: Array<{ role: string; content: string }>
  system?: string
  max_tokens: number
}

/**
 * 运行守卫（生产与测试共用）：runId 不等于当前轮的回调直接丢弃。
 * 返回 true 表示该回调被接受。
 */
export function acceptColumnUpdate(runId: number, currentRunId: number): boolean {
  return runId === currentRunId
}

/** 把单列更新合并进列数组（找不到目标列则原样返回）。 */
export function mergeColumn(columns: RunColumn[], column: RunColumn): RunColumn[] {
  const index = columns.findIndex((item) => item.target === column.target)
  if (index === -1) return columns
  const next = [...columns]
  next[index] = column
  return next
}

/** 单个目标的传输（默认走真实 API；测试注入延迟/错误）。 */
export type ExecuteFn = (
  target: string,
  body: PlaygroundRunBody,
) => Promise<PlaygroundExecuteResponse>

export const defaultExecute: ExecuteFn = (target, body) =>
  api.playgroundExecute({ target, ...body })

/**
 * 对每个目标独立 await，逐列回调 onColumn。
 * 返回各自的 Promise（全部 settle 时本轮结束）；回调携带 runId，
 * UI 端用 acceptColumnUpdate 决定是否应用。
 */
export function runTargets(options: {
  targets: Array<{ target: string; label: string }>
  body: PlaygroundRunBody
  runId: number
  execute?: ExecuteFn
  onColumn: (runId: number, column: RunColumn) => void
}): Array<Promise<void>> {
  const execute = options.execute ?? defaultExecute
  return options.targets.map(({ target, label }) => {
    options.onColumn(options.runId, {
      target,
      label,
      running: true,
      abandoned: false,
      slow: false,
      result: null,
      error: null,
    })
    return execute(target, options.body)
      .then((result) => {
        options.onColumn(options.runId, {
          target,
          label,
          running: false,
          abandoned: false,
          slow: false,
          result,
          error: null,
        })
      })
      .catch((error: unknown) => {
        options.onColumn(options.runId, {
          target,
          label,
          running: false,
          abandoned: false,
          slow: false,
          result: null,
          error: error instanceof Error ? error.message : String(error),
        })
      })
  })
}

/** 停止等待：把运行中的列改为"已停止等待"（前端放弃，非取消上游）。 */
export function abandonColumns(columns: RunColumn[]): RunColumn[] {
  return columns.map((column) =>
    column.running ? { ...column, running: false, abandoned: true, slow: false } : column,
  )
}

/** 给运行中的列打"等待较久"提示。 */
export function markSlowColumns(columns: RunColumn[]): RunColumn[] {
  return columns.map((column) => (column.running ? { ...column, slow: true } : column))
}

/**
 * 列的呈现结论：`ok: false` / `result.error` 非空的响应不是成功。
 */
export function columnOutcome(
  column: RunColumn,
): 'running' | 'abandoned' | 'failed' | 'succeeded' {
  if (column.running) return 'running'
  if (column.abandoned) return 'abandoned'
  if (column.error !== null) return 'failed'
  if (column.result === null) return 'failed'
  return column.result.ok && column.result.error === null ? 'succeeded' : 'failed'
}

/**
 * TPS 口径（§9.1 脚注）：输出 token / 总耗时（含排队与网络），非纯生成速率。
 * 无输出或无耗时时返回 null（显示 —，不用 0 充数）。
 */
export function tpsOf(result: PlaygroundExecuteResponse): number | null {
  if (result.output_tokens === 0 || result.duration_ms === 0) return null
  return (result.output_tokens * 1000) / result.duration_ms
}

/**
 * 对比结论（§9.1）：只在 ≥2 个成功列时给出；只比有数据的列。
 * "最快"指端到端总耗时最短，不是生成速度最快。
 */
export function compareConclusion(columns: RunColumn[]): {
  fastestFirstEvent: string | null
  shortestTotalMs: string | null
} {
  const usable = columns.filter(
    (column) => columnOutcome(column) === 'succeeded' && column.result !== null,
  )
  if (usable.length < 2) return { fastestFirstEvent: null, shortestTotalMs: null }
  let fastestFirstEvent: string | null = null
  let firstEventMs = Number.POSITIVE_INFINITY
  let shortestTotalMs: string | null = null
  let totalMs = Number.POSITIVE_INFINITY
  for (const column of usable) {
    const result = column.result!
    if (result.ttft_ms !== null && result.ttft_ms < firstEventMs) {
      firstEventMs = result.ttft_ms
      fastestFirstEvent = column.label
    }
    if (result.duration_ms > 0 && result.duration_ms < totalMs) {
      totalMs = result.duration_ms
      shortestTotalMs = column.label
    }
  }
  return { fastestFirstEvent, shortestTotalMs }
}
