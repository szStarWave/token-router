/** 页面骨架：侧栏 + 标题栏 + 内容。 */
import { Outlet, useRouterState } from '@tanstack/react-router'
import { Sidebar } from './Sidebar'
import { TitleBar } from './TitleBar'
import { Toast } from '../common/Toast'
import { useLiveEvents } from '../../hooks/useLiveEvents'

const PAGE_TITLES: Record<string, string> = {
  overview: '总览',
  providers: 'Providers',
  quotas: '配额',
  models: '候选模型',
  'virtual-models': '虚拟模型',
  requests: '请求',
  stats: '统计',
  logs: '日志',
  playground: 'Playground',
  onboarding: '接入',
  tokens: 'Token',
  diagnostics: '诊断',
  settings: '设置',
}

export function AppLayout() {
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const page = pathname.replace(/^\//, '').split('/')[0] || 'overview'
  const title = PAGE_TITLES[page] ?? '总览'
  // SSE 实时刷新（Tauri 下桥接；轮询兜底）。
  useLiveEvents()

  return (
    <div className="app-shell" id="app">
      <Sidebar />
      <div className="app-divider" />
      <main className="app-main">
        <header className="app-page-header">
          <div className="app-page-title drag-region" id="page-title">
            {title}
          </div>
          <div className="flex items-center gap-0.5">
            <TitleBar />
          </div>
        </header>
        <div className="app-page-content">
          <Outlet />
        </div>
      </main>
      <Toast />
    </div>
  )
}
