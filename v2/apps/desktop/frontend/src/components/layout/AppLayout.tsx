/**
 * 页面骨架：侧栏 + 标题栏 + 内容（product-design.zh.md §4/§7.1）。
 *
 * - 页头标题/描述用模块用户词（§4.1 词汇表）。
 * - 有二级标签页的模块（应用/供应商/路由组/活动/网关）在页头渲染 oc-seg，
 *   用 Link 实现——路由本身就是页，不拆嵌套路由；旧 path 直达对应标签。
 * - 拖拽窗口控制保持单套：页头整条 drag-region，TitleBar 按钮 no-drag。
 * - 全局草稿状态条（DraftStatusBar）挂在标题右侧、二级导航左侧；无待办时自渲染 null。
 * - 全局关闭守卫（useDraftCloseGuard）：有未保存草稿时，关窗前确认（Tauri +
 *   浏览器 beforeunload），从配置页切走也生效（守卫读全局 store，不依赖当前页）。
 * - 配置热加载：保存后运行中目录立即替换；只有 [server] 段变化才提示重启，
 *   由页内 ConfigSaveBar 提供「重启网关」。
 */
import { Outlet, useRouterState } from '@tanstack/react-router'
import { Link } from '@tanstack/react-router'
import { Sidebar } from './Sidebar'
import { TitleBar } from './TitleBar'
import { Toast } from '../common/Toast'
import { DraftStatusBar } from '../config/DraftStatusBar'
import { useLiveEvents } from '../../hooks/useLiveEvents'
import { useDraftCloseGuard } from '../../hooks/useDraftCloseGuard'
import { findModule, tabsOf } from './nav'

/** 模块标题（用户词）。页头不写描述句：结构与状态自己说明。 */
const MODULE_TITLES: Record<string, string> = {
  apps: '智能体',
  providers: '供应商',
  groups: '路由组',
  activity: '活动',
  usage: '用量',
  gateway: '网关',
  settings: '设置',
}

/** 不在主导航、但保留直达的旧页面标题。 */
const LEGACY_TITLES: Record<string, string> = {
  '/overview': '首页',
  '/playground': '模型测试',
}

export function AppLayout() {
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const module = findModule(pathname)
  const title = module ? MODULE_TITLES[module.id] : (LEGACY_TITLES[pathname] ?? 'Token Router')
  const tabs = tabsOf(module)

  // SSE 实时刷新（Tauri 下桥接；轮询兜底）。
  useLiveEvents()
  // 全局关闭守卫：关窗前确认，不丢 dirty/valid/validating/conflict/applying 草稿。
  useDraftCloseGuard()

  // 「待重启」不再靠 digest 轮询推断：热加载后运行中 digest 立即等于磁盘，
  // 只有真正改了 [server] 段才 pending，由页内保存栏「重启网关」清除。

  return (
    <div className="app-shell" id="app">
      <Sidebar />
      <div className="app-divider" />
      <main className="app-main">
        <header className="app-page-header">
          <div
            className="flex-1 min-w-0 flex items-center gap-0.5 drag-region"
            id="page-title"
          >
            <span className="text-section font-semibold tracking-tight truncate">{title}</span>
          </div>
          {/* 全局草稿状态条（标题右侧、二级导航左侧；无待办时渲染 null） */}
          <div className="no-drag flex items-center">
            <DraftStatusBar />
          </div>
          {tabs && (
            <nav className="no-drag oc-seg" aria-label={`${title}二级导航`}>
              {tabs.map((tab) => {
                const active = pathname === tab.to || pathname.startsWith(`${tab.to}/`)
                return (
                  <Link
                    key={tab.to}
                    to={tab.to}
                    className={active ? 'oc-seg-item-active' : 'oc-seg-item'}
                    aria-current={active ? 'page' : undefined}
                  >
                    {tab.label}
                  </Link>
                )
              })}
            </nav>
          )}
          <div className="flex items-center gap-0.5 no-drag">
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
