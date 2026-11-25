/**
 * Playground（product-design §9.1 / Phase D1 前端项）：
 *
 * - 目标 ≤4 并行；默认从当前默认模型入口单目标起步（一次初始化），
 *   添加对比目标是明确操作；
 * - 逐列独立完成、独立渲染；「停止等待」= 前端放弃等待，不是取消上游；
 * - `ok: false` 的响应不是成功，列显示失败；
 * - 计时口径：首事件延迟 = 首个流式事件；输出速度 = 输出 token / 总耗时；
 * - 目标来自磁盘配置，未重启网关时运行时还没有这些候选：
 *   config revision 与运行时 digest 不一致时提示"待重启生效"并禁用运行；
 * - prompt / system / max_tokens 不落任何本机存储（关闭即丢）。
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import type { StatusResponse } from '../api/types'
import { useStatusQuery, useConfigQuery } from '../queries/admin'
import { EmptyState, ErrorState, Loading } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import {
  abandonColumns,
  acceptColumnUpdate,
  columnOutcome,
  compareConclusion,
  markSlowColumns,
  mergeColumn,
  runTargets,
  tpsOf,
  WAIT_HINT_MS,
  type RunColumn,
} from '../lib/playground-run'

const MAX_TARGETS = 4

interface TargetOption {
  value: string
  label: string
  kind: '模型入口' | '物理候选'
}

function extractTargets(document: unknown): TargetOption[] {
  if (!document || typeof document !== 'object') return []
  const doc = document as {
    virtual_models?: Array<{ id?: string; name?: string; enabled?: boolean }>
    candidates?: Array<{ id?: string; model?: string; enabled?: boolean }>
  }
  const options: TargetOption[] = []
  for (const model of doc.virtual_models ?? []) {
    // 停用入口在运行时不存在：列出来只会让用户测一个必然失败的请求。
    if (!model.id || model.enabled === false) continue
    options.push({
      value: `virtual:${model.id}`,
      label: model.name ? `${model.id}（${model.name}）` : model.id,
      kind: '模型入口',
    })
  }
  for (const candidate of doc.candidates ?? []) {
    if (!candidate.id || candidate.enabled === false) continue
    options.push({
      value: `candidate:${candidate.id}`,
      label: candidate.model ? `${candidate.id}（${candidate.model}）` : candidate.id,
      kind: '物理候选',
    })
  }
  return options
}

function formatMs(ms: number): string {
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`
}

/** `#/playground?target=candidate:xxx` → 初始目标（供应商目录「测试」入口）。 */
function initialTargetFromHash(): string[] | null {
  if (typeof window === 'undefined') return null
  const hash = window.location.hash
  const queryAt = hash.indexOf('?')
  if (queryAt < 0) return null
  const target = new URLSearchParams(hash.slice(queryAt + 1)).get('target')
  return target ? [target] : null
}

export function PlaygroundPage() {
  const configQuery = useConfigQuery()
  const statusQuery = useStatusQuery()
  const [prompt, setPrompt] = useState('')
  const [system, setSystem] = useState('')
  const [showSystem, setShowSystem] = useState(false)
  const [maxTokens, setMaxTokens] = useState(512)
  const [columns, setColumns] = useState<RunColumn[]>([])
  const [running, setRunning] = useState(false)
  const runIdRef = useRef(0)
  const slowTimerRef = useRef<number | null>(null)

  const options = useMemo(
    () => extractTargets(configQuery.data?.document),
    [configQuery.data],
  )

  // 目标来自磁盘配置；与运行时 digest 不一致说明候选还没重启生效。
  const config = configQuery.data
  const pendingRestart =
    config !== undefined && statusQuery.data !== undefined &&
    config.revision !== statusQuery.data.config_digest

  // 默认单目标 + 失效清理：渲染期派生（不加 effect、不读 ref）。
  // userSelected === null 表示用户还没动过目标：默认取当前模型入口；
  // 一旦用户操作（含全部取消）就固定为用户的选择，refetch 不会复原默认值。
  const [userSelected, setUserSelected] = useState<string[] | null>(initialTargetFromHash)
  const optionsById = useMemo(
    () => new Map(options.map((option) => [option.value, option])),
    [options],
  )
  const status = statusQuery.data as StatusResponse | undefined
  const defaultTarget = (() => {
    const preferred = status?.default_virtual_model
      ? `virtual:${status.default_virtual_model}`
      : null
    if (preferred && optionsById.has(preferred)) return preferred
    const firstEntry = options.find((option) => option.kind === '模型入口')
    return firstEntry?.value ?? null
  })()
  const selected = (userSelected ?? (defaultTarget === null ? [] : [defaultTarget])).filter(
    (value) => optionsById.has(value),
  )

  function toggleTarget(target: string) {
    setUserSelected(() => {
      if (selected.includes(target)) {
        return selected.filter((item) => item !== target)
      }
      if (selected.length >= MAX_TARGETS) return selected
      return [...selected, target]
    })
  }

  useEffect(() => {
    return () => {
      if (slowTimerRef.current !== null) window.clearTimeout(slowTimerRef.current)
      runIdRef.current += 1
    }
  }, [])

  function updateColumn(runId: number, column: RunColumn) {
    if (!acceptColumnUpdate(runId, runIdRef.current)) return
    setColumns((current) => mergeColumn(current, column))
  }

  async function run() {
    if (prompt.trim() === '' || selected.length === 0 || pendingRestart) return
    if (slowTimerRef.current !== null) window.clearTimeout(slowTimerRef.current)
    const runId = runIdRef.current + 1
    runIdRef.current = runId
    setRunning(true)
    // 快照当前选择；运行中列表不再响应勾选变化。
    const targets = selected.map((target) => ({
      target,
      label: options.find((option) => option.value === target)?.label ?? target,
    }))
    setColumns(
      targets.map((item) => ({
        ...item,
        running: true,
        abandoned: false,
        slow: false,
        result: null,
        error: null,
      })),
    )
    slowTimerRef.current = window.setTimeout(() => {
      setColumns((current) => markSlowColumns(current))
    }, WAIT_HINT_MS)

    const body = {
      messages: [{ role: 'user', content: prompt }],
      system: system.trim() === '' ? undefined : system,
      max_tokens: maxTokens,
    }
    await Promise.all(
      runTargets({ targets, body, runId, onColumn: updateColumn }),
    )
    if (runId === runIdRef.current) {
      setRunning(false)
      if (slowTimerRef.current !== null) {
        window.clearTimeout(slowTimerRef.current)
        slowTimerRef.current = null
      }
    }
  }

  function stopWaiting() {
    // 停止等待：前端放弃等待，不是取消上游。上游请求可能继续并产生费用。
    if (slowTimerRef.current !== null) {
      window.clearTimeout(slowTimerRef.current)
      slowTimerRef.current = null
    }
    runIdRef.current += 1
    setColumns((current) => abandonColumns(current))
    setRunning(false)
  }

  const conclusion = compareConclusion(columns)
  const anyRunning = columns.some((column) => columnOutcome(column) === 'running')
  const showConclusion =
    !anyRunning &&
    conclusion.fastestFirstEvent !== null

  return (
    <div className="app-page flex-1">
      <section className="app-card" aria-label="目标选择">
        <div className="flex items-center gap-2 flex-wrap">
          <h2 className="app-card-title">目标</h2>
          <span className="flex-1" />
          <span className="text-meta text-muted-foreground">
            已选 {selected.length}/{MAX_TARGETS}
          </span>
        </div>
        {configQuery.isPending ? (
          <Loading label="加载配置…" />
        ) : configQuery.isError ? (
          <ErrorState
            message={configQuery.error.message}
            onRetry={() => void configQuery.refetch()}
          />
        ) : options.length === 0 ? (
          <EmptyState
            title="没有可测试的目标"
            action={
              <a href="#/providers" className="oc-btn-default oc-btn-sm">
                去模型来源页
              </a>
            }
          />
        ) : (
          <>
            <ul className="grid grid-cols-2 gap-1 max-h-64 overflow-y-auto max-[900px]:grid-cols-1">
              {options.map((option) => (
                <li key={option.value}>
                  <label className="flex items-center gap-2 text-meta cursor-pointer">
                    <input
                      type="checkbox"
                      checked={selected.includes(option.value)}
                      disabled={running}
                      onChange={() => toggleTarget(option.value)}
                    />
                    <span className="app-pill">{option.kind}</span>
                    <span className="font-mono truncate">{option.label}</span>
                  </label>
                </li>
              ))}
            </ul>
            <details>
              <summary className="text-meta text-muted-foreground cursor-pointer">
                目标类型与计费
              </summary>
              <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-meta">
                <dt className="text-muted-foreground">模型入口</dt>
                <dd>走正常路由，计入请求账本</dd>
                <dt className="text-muted-foreground">物理候选</dt>
                <dd>绕过路由直达上游，不计路由决策</dd>
                <dt className="text-muted-foreground">费用</dt>
                <dd>两者都是真实上游调用，都可能产生上游费用</dd>
              </dl>
            </details>
          </>
        )}
        {pendingRestart && (
          <div className="app-banner app-banner-warn text-meta">
            网关未重启，新目标在运行时不存在，测试请求会失败。
          </div>
        )}
      </section>

      <section className="app-card" aria-label="测试输入">
        <div className="app-field">
          <span>
            System（可选）
            <button
              type="button"
              className="oc-btn-ghost oc-btn-xs ml-2"
              onClick={() => setShowSystem((value) => !value)}
            >
              {showSystem ? '收起' : '展开'}
            </button>
          </span>
          {showSystem && (
            <textarea
              className="oc-field app-field-light min-h-16 font-mono text-meta"
              value={system}
              onChange={(event) => setSystem(event.target.value)}
              placeholder="You are a helpful assistant."
            />
          )}
        </div>
        <label className="app-field">
          <span>输入</span>
          <textarea
            className="oc-field app-field-light min-h-28 font-mono text-meta"
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            placeholder="写一段 Rust 的快速排序，并解释复杂度"
          />
        </label>
        <div className="app-card-actions">
          <label className="app-field">
            <span>max_tokens</span>
            <input
              className="oc-field app-field-light h-8 w-24"
              type="number"
              min={1}
              max={8192}
              value={maxTokens}
              onChange={(event) => setMaxTokens(Number(event.target.value) || 512)}
            />
          </label>
          <button
            type="button"
            className="oc-btn-primary"
            disabled={
              running || prompt.trim() === '' || selected.length === 0 || pendingRestart
            }
            title="当前实现为聚合返回（非逐 token 流式）：等待完整结果后一次显示。"
            onClick={() => void run()}
          >
            <Icon name="start" />
            {running ? '运行中…' : `运行 ${selected.length} 个目标`}
          </button>
          {running && (
            <button type="button" className="oc-btn-default" onClick={stopWaiting}>
              <Icon name="stop" />
              停止等待
            </button>
          )}
        </div>
      </section>

      {columns.length > 0 && (
        <section
          className="grid gap-3"
          style={{ gridTemplateColumns: `repeat(${Math.min(columns.length, 2)}, minmax(0, 1fr))` }}
          aria-label="测试结果"
        >
          {columns.map((column) => (
            <ResultColumn key={column.target} column={column} />
          ))}
        </section>
      )}

      {showConclusion && (
        <section className="app-card" aria-label="对比结论">
          <div
            className="flex items-center gap-2 flex-wrap text-meta"
            title="只比较成功的列；快不代表质量更优。"
          >
            <span>
              最快首事件：<span className="font-mono">{conclusion.fastestFirstEvent}</span>
            </span>
            {conclusion.shortestTotalMs !== null && (
              <span>
                最快总耗时：<span className="font-mono">{conclusion.shortestTotalMs}</span>
              </span>
            )}
          </div>
        </section>
      )}
    </div>
  )
}

function ResultColumn({ column }: { column: RunColumn }) {
  const outcome = columnOutcome(column)
  const result = column.result
  return (
    <article className="app-card">
      <div className="flex items-center gap-2 flex-wrap">
        <h3 className="app-card-title text-ui-label flex-1 truncate">{column.label}</h3>
        {outcome === 'running' ? (
          <span className="app-pill app-pill-warn">
            {column.slow ? '仍在等待完整结果…' : '运行中'}
          </span>
        ) : outcome === 'abandoned' ? (
          <span className="app-pill app-pill-warn">已停止等待</span>
        ) : outcome === 'failed' ? (
          <span className="app-pill app-pill-error">失败</span>
        ) : (
          <span className="app-pill app-pill-ok">完成</span>
        )}
      </div>
      {outcome === 'abandoned' && (
        <p className="app-banner app-banner-warn text-meta">
          已停止等待：这只是前端放弃等待，上游请求可能仍在执行并产生费用。
        </p>
      )}
      {outcome === 'running' && column.slow && (
        <p className="text-meta text-muted-foreground">
          超过 30 秒未返回，可停止等待。
        </p>
      )}
      {result && outcome !== 'abandoned' && (
        <dl className="app-kv">
          <div className="app-kv-item">
            <dt className="app-kv-key">首事件延迟（TTFT 口径）</dt>
            <dd className="app-kv-value">
              {result.ttft_ms === null ? '—' : formatMs(result.ttft_ms)}
            </dd>
          </div>
          <div className="app-kv-item">
            <dt className="app-kv-key">总耗时</dt>
            <dd className="app-kv-value">{formatMs(result.duration_ms)}</dd>
          </div>
          <div className="app-kv-item">
            <dt className="app-kv-key">tokens (in/out)</dt>
            <dd className="app-kv-value">
              {result.input_tokens} / {result.output_tokens}
            </dd>
          </div>
          <div className="app-kv-item">
            <dt className="app-kv-key">输出速度（TPS 口径）</dt>
            <dd className="app-kv-value">
              {tpsOf(result) === null ? '—' : `${tpsOf(result)!.toFixed(1)} tok/s`}
            </dd>
          </div>
        </dl>
      )}
      {outcome === 'failed' && column.error && (
        <p className="app-banner app-banner-err text-meta">{column.error}</p>
      )}
      {outcome === 'failed' && result?.error && (
        <p className="app-banner app-banner-err text-meta">{result.error}</p>
      )}
      {result && outcome === 'succeeded' && (
        <pre className="app-code-block max-h-72 overflow-auto whitespace-pre-wrap">
          {result.text || '（空响应）'}
        </pre>
      )}
    </article>
  )
}
