/**
 * 上下文窗口（活动页）：已用上下文、提示词缓存命中率、token 分布。
 *
 * 参照 Magpie 的「上下文窗口」卡，但口径按我们的账本字段重画：
 * 我们只区分 输入 / 输出 / 缓存读 / 缓存写 四类 token，没有「系统 / 工具 /
 * 技能 / 记忆 / 文件」那套语义分类，就不画那套分类（不编口径）。
 * 上下文上限取路由组配置的 context_window；没配就只显示已用量。
 */
import { formatCompactNum } from '../../lib/format-number'

export interface TokenTotals {
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
}

export interface ContextPanelProps {
  contextWindow: number | null
  tokens: TokenTotals
  cacheHitRatio: number | null
  /** 统计窗内的会话数（usage by session）。 */
  sessions: number
}

export function ContextPanel({ contextWindow, tokens, cacheHitRatio, sessions }: ContextPanelProps) {
  const used = tokens.input + tokens.cacheRead + tokens.cacheWrite + tokens.output
  const free = contextWindow ? Math.max(contextWindow - used, 0) : 0
  const ratio = contextWindow && contextWindow > 0 ? Math.min(used / contextWindow, 1) : null

  const segments = [
    { key: 'input', label: '输入', value: tokens.input, className: 'app-token-seg-input' },
    { key: 'cacheRead', label: '缓存读', value: tokens.cacheRead, className: 'app-token-seg-cache-read' },
    { key: 'cacheWrite', label: '缓存写', value: tokens.cacheWrite, className: 'app-token-seg-cache-write' },
    { key: 'output', label: '输出', value: tokens.output, className: 'app-token-seg-output' },
  ]

  const scale = contextWindow ?? Math.max(used, 1)

  return (
    <div className="flex flex-col gap-4">
      <div className="app-context-top">
        <div className="flex flex-col gap-1">
          <span className="text-meta text-muted-foreground">已用上下文</span>
          <span className="text-3xl leading-tight font-semibold tracking-tight tabular-nums">
            {formatCompactNum(used)}
          </span>
          <span className="text-meta text-muted-foreground">
            {contextWindow
              ? `${formatCompactNum(contextWindow)} token 上限 · 占比 ${(ratio === null ? 0 : ratio * 100).toFixed(1)}%`
              : '路由组未配置上下文窗口'}
          </span>
        </div>
        <div className="flex flex-col gap-1">
          <span className="text-meta text-muted-foreground">提示词缓存命中率</span>
          <span className="text-3xl leading-tight font-semibold tracking-tight tabular-nums">
            {cacheHitRatio === null ? '—' : `${(cacheHitRatio * 100).toFixed(2)}%`}
          </span>
          <span className="text-meta text-muted-foreground">
            {contextWindow
              ? `剩余空间 ${formatCompactNum(free)} token`
              : `${sessions} 个会话`}
          </span>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <div className="app-token-bar">
          {segments.map((segment) => {
            const width = (segment.value / scale) * 100
            if (width <= 0) return null
            return (
              <div
                key={segment.key}
                className={segment.className}
                style={{ width: `${Math.min(width, 100)}%` }}
                title={`${segment.label} ${segment.value}`}
              />
            )
          })}
          {contextWindow && free > 0 && (
            <div
              className="app-token-seg-free"
              style={{ width: `${Math.min((free / scale) * 100, 100)}%` }}
              title={`剩余 ${free}`}
            />
          )}
        </div>
        <div className="flex items-center gap-3 flex-wrap text-meta text-muted-foreground">
          {segments.map((segment) => (
            <span key={segment.key} className="inline-flex items-center gap-1.5">
              <span className={segment.className} aria-hidden />
              {segment.label} {formatCompactNum(segment.value)}
            </span>
          ))}
          {contextWindow && (
            <span className="inline-flex items-center gap-1.5">
              <span className="app-token-seg-free" aria-hidden />
              剩余空间 {formatCompactNum(free)}
            </span>
          )}
        </div>
      </div>
    </div>
  )
}
