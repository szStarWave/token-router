import { Outlet, useRouterState } from '@tanstack/react-router'
import { Sidebar } from './Sidebar'
import { TitleBar } from './TitleBar'
import { Toast } from '../common/Toast'
import { OnboardingProvider } from '../onboarding/OnboardingProvider'
import { useI18n } from '../../hooks/useI18n'
import { useEdgeSetupSync } from '../../hooks/useEdgeSetupSync'
import { useAutoRefreshBalance } from '../../hooks/useAutoRefreshBalance'

const PAGE_TITLE_KEYS: Record<string, string> = {
  overview: 'page.overview',
  upstream: 'page.upstream',
  upstream_edge: 'page.upstreamEdge',
  upstream_cloud: 'page.upstreamCloud',
  routing: 'page.routing',
  stats: 'page.stats',
  agents: 'page.agents',
  logs: 'page.logs',
  settings: 'page.settings',
}

export function AppLayout() {
  const { t } = useI18n()
  useEdgeSetupSync()
  useAutoRefreshBalance()
  const pathname = useRouterState({ select: (s) => s.location.pathname })

  const pageTitle = (() => {
    const upstreamMatch = pathname.match(/^\/upstream\/(\w+)/)
    if (upstreamMatch) {
      const navId = upstreamMatch[1]
      const key = navId === 'edge' || navId === 'cloud' ? `upstream_${navId}` : 'upstream'
      return t(PAGE_TITLE_KEYS[key] ?? 'page.upstream')
    }
    const page = pathname.replace(/^\//, '').split('/')[0] || 'overview'
    return t(PAGE_TITLE_KEYS[page] ?? 'page.overview')
  })()

  return (
    <OnboardingProvider>
      <div className="app" id="app">
        <Sidebar />
        <div className="divider-v" />
        <main className="main">
          <header className="page-header">
            <div className="page-title window-drag" id="page-title">
              {pageTitle}
            </div>
            <div className="page-header-right">
              <TitleBar />
            </div>
          </header>
          <div className="page-content">
            <Outlet />
          </div>
        </main>
      </div>
      <Toast />
    </OnboardingProvider>
  )
}
