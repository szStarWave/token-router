/**
 * v2 导航（对齐设计原型 .zcode/prototype-magpie/index.html）：
 * 7 个一级任务入口，单组平铺；底部是「网关状态」卡片与主题切换按钮。
 * 结构定义见 ./nav.ts；选中态用 selection 色（"当前是"），不用 primary。
 *
 * 状态卡语义（诚实口径）：绿 = 管理服务已连接（status 查询成功），
 * 黄 = 查询失败，灰 = 连接中。不暗示"路由全部就绪"——
 * 路由可用性由首页七态与诊断页如实呈现。
 */
import type { ReactNode } from 'react'
import { Link, useRouterState } from '@tanstack/react-router'
import { useConfigQuery, useStatusQuery } from '../../queries/admin'
import { useAppStore } from '../../stores/appStore'
import { useConfigDraftStore } from '../../stores/configDraftStore'
import { Icon } from '../common/Icon'
import { SIDEBAR_GROUPS, moduleById, type ModuleId } from './nav'

function NavItem({ id, active }: { id: ModuleId; active: boolean }) {
  const item = moduleById(id)
  return (
    <li>
      <Link
        to={item.root}
        className={active ? 'app-nav-item app-nav-item-active' : 'app-nav-item'}
        activeOptions={{ exact: false }}
        aria-current={active ? 'page' : undefined}
      >
        <Icon name={item.icon} className="size-4 shrink-0" />
        {item.label}
      </Link>
    </li>
  )
}

/** 状态卡的一行：左侧字段名、右侧值。 */
function StatusRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2 text-meta">
      <span className="text-muted-foreground">{label}</span>
      <span className="flex items-center gap-1.5 min-w-0 truncate">{children}</span>
    </div>
  )
}

export function Sidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const statusQuery = useStatusQuery()
  const configQuery = useConfigQuery()
  const restartPending = useConfigDraftStore((s) => s.restartPending)
  const themePref = useAppStore((s) => s.themePref)
  const setThemePref = useAppStore((s) => s.setThemePref)

  const connected = statusQuery.isSuccess
  const failed = statusQuery.isError
  const status = connected ? statusQuery.data : undefined

  const serviceLabel = failed ? '连接失败' : connected ? '已连接' : '连接中'
  const dotClass = failed
    ? 'app-status-dot bg-status-warning'
    : connected
      ? 'app-status-dot bg-status-success'
      : 'app-status-dot opacity-60'
  const listenLabel = status?.listen ?? '—'
  const digestLabel = status ? status.config_digest.slice(0, 8) : '—'
  // 磁盘 revision 与运行中 digest 不一致（本会话 apply、CLI 手改、rollback 都算）
  // 就说明“有改动未重启”，不只看本会话的 apply 标记。
  const diskRevision = configQuery.data?.revision
  const pendingRestart =
    restartPending ||
    (diskRevision !== undefined &&
      status?.config_digest !== undefined &&
      diskRevision !== status.config_digest)
  const pendingLabel = pendingRestart ? '重启后生效' : '无'

  const toggleTheme = () => {
    const effective = document.documentElement.dataset.effectiveTheme
    const isDark = effective === 'dark' || (effective === undefined && themePref === 'dark')
    setThemePref(isDark ? 'light' : 'dark')
  }

  const renderItem = (id: ModuleId) => {
    const item = moduleById(id)
    const active = item.paths.some(
      (path) => pathname === path || pathname.startsWith(`${path}/`),
    )
    return <NavItem key={id} id={id} active={active} />
  }

  return (
    <nav className="app-sidebar" aria-label="主导航">
      <div className="app-brand">
        <span className="app-brand-mark" aria-hidden />
        <span className="app-brand-name">Token Router</span>
      </div>

      {SIDEBAR_GROUPS.map((group) => (
        <ul className="app-nav" key={group.join('-')}>
          {group.map(renderItem)}
        </ul>
      ))}

      <div className="mt-auto flex flex-col gap-1.5 rounded-md p-2.5 bg-surface-elevated">
        <div className="text-ui-label font-semibold">网关状态</div>
        <StatusRow label="服务">
          <span className={dotClass} aria-hidden />
          {serviceLabel}
        </StatusRow>
        <StatusRow label="监听">
          <span className="font-mono truncate">{listenLabel}</span>
        </StatusRow>
        <StatusRow label="配置">
          <span className="font-mono">{digestLabel}</span>
          {connected && <span className="text-muted-foreground">运行中</span>}
        </StatusRow>
        <StatusRow label="待发布">
          <span className={pendingRestart ? 'text-status-warning' : 'text-muted-foreground'}>
            {pendingLabel}
          </span>
        </StatusRow>
      </div>

      <button type="button" className="oc-btn-default mt-1 w-full" onClick={toggleTheme}>
        切换亮暗主题
      </button>
    </nav>
  )
}
