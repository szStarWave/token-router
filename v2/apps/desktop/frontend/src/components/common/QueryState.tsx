/**
 * 统一加载/空/错状态（P4-061）：所有页面复用。
 */
import type { ReactNode } from 'react'

export function Loading({ label = '加载中…' }: { label?: string }) {
  return (
    <div className="state state-loading" role="status">
      <div className="state-spinner" aria-hidden />
      <span>{label}</span>
    </div>
  )
}

export function EmptyState({
  title,
  hint,
  action,
}: {
  title: string
  hint?: string
  action?: ReactNode
}) {
  return (
    <div className="state state-empty">
      <div className="state-title">{title}</div>
      {hint && <div className="state-hint">{hint}</div>}
      {action && <div className="state-action">{action}</div>}
    </div>
  )
}

export function ErrorState({
  message,
  onRetry,
}: {
  message: string
  onRetry?: () => void
}) {
  return (
    <div className="state state-error" role="alert">
      <div className="state-title">出错了</div>
      <div className="state-hint">{message}</div>
      {onRetry && (
        <button type="button" className="btn" onClick={onRetry}>
          重试
        </button>
      )}
    </div>
  )
}

/** TanStack Query 三态包装。 */
export function QueryState({
  loading,
  error,
  empty,
  children,
  onRetry,
}: {
  loading: boolean
  error: unknown
  empty?: boolean
  children: ReactNode
  onRetry?: () => void
}) {
  if (loading) return <Loading />
  if (error) {
    const message =
      error instanceof Error
        ? error.message
        : typeof error === 'string'
          ? error
          : '请求失败'
    return <ErrorState message={message} onRetry={onRetry} />
  }
  if (empty) {
    return (
      <EmptyState
        title="暂无数据"
        hint="v2 从空账本开始；配置候选并发起请求后这里会出现数据。"
      />
    )
  }
  return <>{children}</>
}

/** 错误消息提取。 */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  return String(error)
}
