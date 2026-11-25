/**
 * SSE 实时刷新（§M6-082）：Tauri 下经 Rust 事件桥（admin://event）；
 * 断线由桥自动重连（Last-Event-ID）。轮询仍是兜底（页面隐藏自动暂停）。
 *
 * 用法：在 AppLayout 挂一次 `useLiveEvents()`；QueryClient 失效按事件类型定向。
 */
import { useEffect, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { isTauri } from '../lib/tauri'
import { useQueryClient } from '@tanstack/react-query'
import { usePrefsStore } from '../stores/prefsStore'

export interface AdminEvent {
  id: number
  type: string
  data: Record<string, unknown>
}

/** 订阅 admin 事件并按类型失效查询；返回最近一条事件（供状态栏显示）。 */
export function useLiveEvents(): AdminEvent | null {
  const queryClient = useQueryClient()
  const [latest, setLatest] = useState<AdminEvent | null>(null)
  // 轻量模式：不建 SSE 长连接，页面靠轮询兜底（省电、少一条后台连接）。
  const liteMode = usePrefsStore((s) => s.liteMode)

  useEffect(() => {
    if (!isTauri() || liteMode) return
    let unlisten: UnlistenFn | null = null
    let cancelled = false

    // 先让 Rust 侧建立 SSE 连接，再监听桥事件。
    void (async () => {
      const { invoke } = await import('@tauri-apps/api/core')
      try {
        await invoke('admin_events_subscribe')
      } catch {
        return // 服务未启动：轮询兜底
      }
      if (cancelled) return
      unlisten = await listen<AdminEvent>('admin://event', (event) => {
        const payload = event.payload
        setLatest(payload)
        // 按事件类型精确失效（§7.2 条款 3）。
        switch (payload.type) {
          case 'request.completed':
            void queryClient.invalidateQueries({ queryKey: ['admin', 'requests'] })
            void queryClient.invalidateQueries({ queryKey: ['admin', 'stats'] })
            // 会话汇总与判官审计都随请求增长，活动页「会话」标签靠这两条刷新。
            void queryClient.invalidateQueries({ queryKey: ['admin', 'sessions'] })
            void queryClient.invalidateQueries({ queryKey: ['admin', 'session-requests'] })
            void queryClient.invalidateQueries({ queryKey: ['admin', 'judge-calls'] })
            break
          case 'config.applied':
            void queryClient.invalidateQueries({ queryKey: ['admin', 'config'] })
            void queryClient.invalidateQueries({ queryKey: ['admin', 'status'] })
            break
          case 'breaker.opened':
            void queryClient.invalidateQueries({ queryKey: ['admin', 'health'] })
            break
          default:
            break
        }
      })
    })()

    return () => {
      cancelled = true
      unlisten?.()
      if (isTauri()) {
        void import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('admin_events_unsubscribe'),
        )
      }
    }
  }, [queryClient, liteMode])

  return latest
}
