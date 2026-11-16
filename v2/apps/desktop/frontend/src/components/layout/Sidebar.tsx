/**
 * v2 导航（§7.3 页面清单）：Overview/Models/Virtual Models/Requests/Stats/
 * Logs/Onboarding/Tokens/Settings/Approvals/Diagnostics。
 */
import { Link, useRouterState } from '@tanstack/react-router'
import { useStatusQuery } from '../../queries/admin'

const NAV_ITEMS = [
  { to: '/overview', label: '总览', id: 'overview' },
  { to: '/models', label: '候选模型', id: 'models' },
  { to: '/virtual-models', label: '虚拟模型', id: 'virtual-models' },
  { to: '/requests', label: '请求', id: 'requests' },
  { to: '/stats', label: '统计', id: 'stats' },
  { to: '/logs', label: '日志', id: 'logs' },
  { to: '/tokens', label: 'Token', id: 'tokens' },
  { to: '/approvals', label: '审批', id: 'approvals' },
  { to: '/diagnostics', label: '诊断', id: 'diagnostics' },
  { to: '/settings', label: '设置', id: 'settings' },
] as const

export function Sidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const statusQuery = useStatusQuery()
  const state = statusQuery.data
  const ready = state !== undefined
  const openBreakers = state?.breakers.open ?? 0

  return (
    <nav className="sidebar" aria-label="主导航">
      <div className="sidebar-brand">
        <span className="brand-dot" aria-hidden />
        <span className="brand-name">Token Router v2</span>
      </div>
      <ul className="nav-list">
        {NAV_ITEMS.map((item) => {
          const active = pathname.startsWith(item.to)
          return (
            <li key={item.id}>
              <Link
                to={item.to}
                className={`nav-item${active ? ' active' : ''}`}
                activeOptions={{ exact: false }}
              >
                {item.label}
                {item.id === 'diagnostics' && openBreakers > 0 && (
                  <span className="nav-badge" aria-label={`${openBreakers} 个熔断打开`}>
                    {openBreakers}
                  </span>
                )}
              </Link>
            </li>
          )
        })}
      </ul>
      <div className="sidebar-footer">
        <span
          className={`service-dot ${ready ? (state?.breakers.open ? 'warn' : 'ok') : 'idle'}`}
          aria-hidden
        />
        <span className="service-text">
          {!ready ? '服务未连接' : state?.default_virtual_model ? `默认: ${state.default_virtual_model}` : '…'}
        </span>
      </div>
    </nav>
  )
}
