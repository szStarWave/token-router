/**
 * 统一加载/空/错状态（P4-061）：所有页面复用。
 *
 * 状态行有预留槽：加载/空/错/完成都在同一块高度里，不让页面跳动。
 */
import type { ReactNode } from 'react'
import { Icon } from './Icon'

export function Loading({ label = '加载中…' }: { label?: string }) {
  return (
    <div className="app-state" role="status">
      <div className="app-spinner" aria-hidden>
        {Array.from({ length: 8 }, (_, index) => (
          <span key={index} className="app-spinner-bar" />
        ))}
      </div>
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
    <div className="app-state">
      <Icon name="empty" className="size-6 opacity-40" />
      <div className="app-state-title">{title}</div>
      {hint && <div className="app-state-hint">{hint}</div>}
      {action && <div className="app-state-action">{action}</div>}
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
    <div className="app-state" role="alert">
      <Icon name="error" className="size-6 text-status-error" />
      <div className="app-state-title app-state-error-title">出错了</div>
      <div className="app-state-hint">{message}</div>
      {onRetry && (
        <button type="button" className="oc-btn-default" onClick={onRetry}>
          <Icon name="refresh" />
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
