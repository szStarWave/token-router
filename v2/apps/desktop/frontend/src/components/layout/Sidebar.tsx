/**
 * v2 导航（§7.3 页面清单）：Overview/Models/Virtual Models/Requests/Stats/
 * Logs/Onboarding/Tokens/Settings/Approvals/Diagnostics。
 *
 * 选中态用 selection（"当前是"），不用 primary（"去做"）——见 DESIGN.md §Colors。
 */
import { Link, useRouterState } from '@tanstack/react-router'
import { useStatusQuery } from '../../queries/admin'
import { Icon, type IconName } from '../common/Icon'

const NAV_ITEMS = [
  { to: '/overview', label: '总览', id: 'overview', icon: 'nav-overview' },
  { to: '/models', label: '候选模型', id: 'models', icon: 'nav-models' },
  { to: '/virtual-models', label: '虚拟模型', id: 'virtual-models', icon: 'nav-virtual-models' },
  { to: '/requests', label: '请求', id: 'requests', icon: 'nav-requests' },
  { to: '/stats', label: '统计', id: 'stats', icon: 'nav-stats' },
  { to: '/logs', label: '日志', id: 'logs', icon: 'nav-logs' },
  { to: '/onboarding', label: '接入', id: 'onboarding', icon: 'nav-onboarding' },
  { to: '/tokens', label: 'Token', id: 'tokens', icon: 'nav-tokens' },
  { to: '/approvals', label: '审批', id: 'approvals', icon: 'nav-approvals' },
  { to: '/diagnostics', label: '诊断', id: 'diagnostics', icon: 'nav-diagnostics' },
  { to: '/settings', label: '设置', id: 'settings', icon: 'nav-settings' },
] as const satisfies ReadonlyArray<{ to: string; label: string; id: string; icon: IconName }>

export function Sidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const statusQuery = useStatusQuery()
  const state = statusQuery.data
  const ready = state !== undefined
  const openBreakers = state?.breakers.open ?? 0

  const dotClass = !ready
    ? 'app-status-dot opacity-60'
    : state?.breakers.open
      ? 'app-status-dot bg-status-warning'
      : 'app-status-dot bg-status-success'

  return (
    <nav className="app-sidebar" aria-label="主导航">
      <div className="app-brand">
        <span className="app-brand-mark" aria-hidden />
        <span className="app-brand-name">Token Router v2</span>
      </div>
      <ul className="app-nav">
        {NAV_ITEMS.map((item) => {
          const active = pathname.startsWith(item.to)
          return (
            <li key={item.id}>
              <Link
                to={item.to}
                className={active ? 'app-nav-item app-nav-item-active' : 'app-nav-item'}
                activeOptions={{ exact: false }}
              >
                <Icon name={item.icon} className="size-4 shrink-0" />
                {item.label}
                {item.id === 'diagnostics' && openBreakers > 0 && (
                  <span className="app-nav-badge" aria-label={`${openBreakers} 个熔断打开`}>
                    {openBreakers}
                  </span>
                )}
              </Link>
            </li>
          )
        })}
      </ul>
      <div className="app-sidebar-footer">
        <span className={dotClass} aria-hidden />
        <span className="truncate">
          {!ready ? '服务未连接' : state?.default_virtual_model ? `默认: ${state.default_virtual_model}` : '…'}
        </span>
      </div>
    </nav>
  )
}
