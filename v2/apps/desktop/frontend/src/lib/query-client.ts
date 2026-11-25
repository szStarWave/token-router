/**
 * 全局 QueryClient 单例。
 *
 * 自动保存调度器（`stores/autosaveController.ts`）在 React 之外运行，
 * 需要在保存/冲突后直接写缓存，因此把 QueryClient 从 `main.tsx` 抽到这里，
 * 由 `main.tsx` 引入并挂到 Provider 上。
 */
import { QueryClient } from '@tanstack/react-query'

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
})
