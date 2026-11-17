/**
 * Playground（v3 P5-D）：对虚拟模型或物理候选发起测试请求。
 *
 * Compare 模式最多 4 个目标并行：同 prompt、各自计时（TTFT/总耗时/TPS），
 * 用来回答「同一个问题哪个模型更合适」。
 */
import { useMemo, useState } from 'react'
import { api } from '../api/client'
import type { PlaygroundExecuteResponse } from '../api/types'
import { useConfigQuery } from '../queries/admin'
import { EmptyState, Loading } from '../components/common/QueryState'

const MAX_TARGETS = 4

interface TargetOption {
  value: string
  label: string
  kind: string
}

interface ColumnState {
  target: string
  label: string
  running: boolean
  result: PlaygroundExecuteResponse | null
  error: string | null
}

function extractTargets(document: unknown): TargetOption[] {
  const doc = document as {
    virtual_models?: Array<{ id?: string; name?: string }>
    candidates?: Array<{ id?: string; model?: string; enabled?: boolean }>
  }
  const options: TargetOption[] = []
  for (const model of doc.virtual_models ?? []) {
    if (!model.id) continue
    options.push({
      value: `virtual:${model.id}`,
      label: model.name ? `${model.id}（${model.name}）` : model.id,
      kind: '虚拟模型',
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

export function PlaygroundPage() {
  const configQuery = useConfigQuery()
  const [prompt, setPrompt] = useState('')
  const [system, setSystem] = useState('')
  const [maxTokens, setMaxTokens] = useState(512)
  const [selected, setSelected] = useState<string[]>([])
  const [columns, setColumns] = useState<ColumnState[]>([])
  const [running, setRunning] = useState(false)

  const options = useMemo(
    () => extractTargets(configQuery.data?.document),
    [configQuery.data],
  )

  function toggle(target: string) {
    setSelected((current) => {
      if (current.includes(target)) {
        return current.filter((item) => item !== target)
      }
      if (current.length >= MAX_TARGETS) return current
      return [...current, target]
    })
  }

  async function run() {
    if (prompt.trim() === '' || selected.length === 0) return
    setRunning(true)
    setColumns(
      selected.map((target) => ({
        target,
        label: options.find((option) => option.value === target)?.label ?? target,
        running: true,
        result: null,
        error: null,
      })),
    )
    const body = {
      messages: [{ role: 'user', content: prompt }],
      system: system.trim() === '' ? undefined : system,
      max_tokens: maxTokens,
    }
    const results = await Promise.all(
      selected.map(async (target) => {
        try {
          const result = await api.playgroundExecute({ target, ...body })
          return { target, result, error: null as string | null }
        } catch (error) {
          return {
            target,
            result: null as PlaygroundExecuteResponse | null,
            error: error instanceof Error ? error.message : String(error),
          }
        }
      }),
    )
    setColumns((current) =>
      current.map((column) => {
        const hit = results.find((item) => item.target === column.target)
        return {
          ...column,
          running: false,
          result: hit?.result ?? null,
          error: hit?.error ?? null,
        }
      }),
    )
    setRunning(false)
  }

  return (
    <div className="app-page-content">
      <div>
        <h1 className="app-card-title">Playground</h1>
        <p className="app-card-sub">
          同 prompt 最多 4 个目标并行（虚拟模型走正常路由；物理候选绕过路由直达上游）。
          计时为端到端实测：TTFT = 首个流式事件，TPS = 输出 token / 总耗时。
        </p>
      </div>

      <section className="app-card" aria-label="测试输入">
        <label className="app-field">
          <span>System（可选）</span>
          <textarea
            className="oc-field app-field-light min-h-16 font-mono text-meta"
            value={system}
            onChange={(event) => setSystem(event.target.value)}
            placeholder="You are a helpful assistant."
          />
        </label>
        <label className="app-field">
          <span>Prompt</span>
          <textarea
            className="oc-field app-field-light min-h-28 font-mono text-meta"
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            placeholder="写一段 Rust 的快速排序，并解释复杂度"
          />
        </label>
        <div className="app-form-row">
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
            disabled={running || prompt.trim() === '' || selected.length === 0}
            onClick={() => void run()}
          >
            {running ? '运行中…' : `运行（${selected.length} 个目标）`}
          </button>
          <span className="text-meta text-muted-foreground">
            已选 {selected.length}/{MAX_TARGETS}
          </span>
        </div>
      </section>

      <section className="app-card" aria-label="目标选择">
        <h2 className="app-card-title">目标</h2>
        {configQuery.isPending ? (
          <Loading label="加载配置…" />
        ) : options.length === 0 ? (
          <EmptyState
            title="没有可用目标"
            hint="先在 Providers 页编译候选，或手动在候选模型页添加。"
          />
        ) : (
          <ul className="grid grid-cols-2 gap-1 max-h-64 overflow-y-auto max-[900px]:grid-cols-1">
            {options.map((option) => (
              <li key={option.value}>
                <label className="flex items-center gap-2 text-meta cursor-pointer">
                  <input
                    type="checkbox"
                    checked={selected.includes(option.value)}
                    onChange={() => toggle(option.value)}
                  />
                  <span className="app-pill">{option.kind}</span>
                  <span className="font-mono truncate">{option.label}</span>
                </label>
              </li>
            ))}
          </ul>
        )}
      </section>

      {columns.length > 0 && (
        <section
          className="grid gap-3"
          style={{ gridTemplateColumns: `repeat(${Math.min(columns.length, 2)}, minmax(0, 1fr))` }}
          aria-label="测试结果"
        >
          {columns.map((column) => (
            <article key={column.target} className="app-card">
              <div className="flex items-center gap-2 flex-wrap">
                <h3 className="app-card-title text-ui-label flex-1 truncate">{column.label}</h3>
                {column.running ? (
                  <span className="app-pill app-pill-warn">运行中</span>
                ) : column.error ? (
                  <span className="app-pill app-pill-error">失败</span>
                ) : (
                  <span className="app-pill app-pill-ok">完成</span>
                )}
              </div>
              {column.result && (
                <dl className="app-kv">
                  <div className="app-kv-item">
                    <dt className="app-kv-key">TTFT</dt>
                    <dd className="app-kv-value">
                      {column.result.ttft_ms === null ? '—' : `${column.result.ttft_ms}ms`}
                    </dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">总耗时</dt>
                    <dd className="app-kv-value">{column.result.duration_ms}ms</dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">tokens (in/out)</dt>
                    <dd className="app-kv-value">
                      {column.result.input_tokens} / {column.result.output_tokens}
                    </dd>
                  </div>
                  <div className="app-kv-item">
                    <dt className="app-kv-key">TPS</dt>
                    <dd className="app-kv-value">
                      {column.result.output_tokens === 0 ||
                      column.result.duration_ms === 0
                        ? '—'
                        : `${(
                            (column.result.output_tokens * 1000) /
                            column.result.duration_ms
                          ).toFixed(1)} tok/s`}
                    </dd>
                  </div>
                </dl>
              )}
              {column.error && (
                <p className="app-banner app-banner-err text-meta">{column.error}</p>
              )}
              {column.result?.error && (
                <p className="app-banner app-banner-err text-meta">{column.result.error}</p>
              )}
              {column.result && (
                <pre className="app-code-block max-h-72 overflow-auto whitespace-pre-wrap">
                  {column.result.text || '（空响应）'}
                </pre>
              )}
            </article>
          ))}
        </section>
      )}
    </div>
  )
}
