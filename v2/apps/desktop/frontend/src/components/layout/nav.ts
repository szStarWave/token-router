/**
 * 导航模块定义（对齐设计原型 .zcode/prototype-magpie/index.html）：
 * 7 个一级任务入口，侧栏只有一组，无底部系统区。
 *
 * 路由合并规则（均为前端呈现层合并，旧 path 不失效）：
 * - 应用吸收 /onboarding（二级标签「手动接入」）
 * - 供应商吸收 /quotas（二级标签「配额」）
 * - 路由组只保留 /virtual-models；旧 /models（高级：候选模型）已删除并重定向到路由组
 * - 活动吸收 /logs（二级标签「日志」）与「会话」视图（完整模型调用记录）
 * - 网关吸收 /tokens、/diagnostics（二级标签「网关密钥」「诊断」）
 *
 * 全部路由保留可达；旧 path 不在主导航平铺，由所在模块的二级标签直达。
 * /overview、/playground 仍注册路由，但不再进主导航。
 * 保持 `as const satisfies`：`to` 保留字面量类型，供 tanstack Router 的
 * 类型化 `Link to` 使用，不要放宽成 `string`。
 */
import type { IconName } from '../common/Icon'

export interface ModuleTab {
  to: string
  label: string
}

export interface ModuleDef {
  id: string
  label: string
  icon: IconName
  /** 主导航指向的一级路径。 */
  root: string
  /** 本模块包含的全部路径（含 root），用于选中态判定。 */
  paths: string[]
  /** 模块内二级标签页（AppLayout 页头 oc-seg）。 */
  tabs?: ModuleTab[]
}

export const MODULES = [
  {
    id: 'apps',
    label: '智能体',
    icon: 'nav-apps',
    root: '/apps',
    paths: ['/apps', '/onboarding'],
    tabs: [
      { to: '/apps', label: '智能体' },
      { to: '/onboarding', label: '手动接入' },
    ],
  },
  {
    id: 'providers',
    label: '供应商',
    icon: 'nav-providers',
    root: '/providers',
    paths: ['/providers', '/quotas'],
    tabs: [
      { to: '/providers', label: '厂商与账号' },
      { to: '/quotas', label: '余额' },
    ],
  },
  {
    id: 'groups',
    label: '路由组',
    icon: 'nav-virtual-models',
    root: '/virtual-models',
    paths: ['/virtual-models'],
  },
  {
    id: 'activity',
    label: '活动',
    icon: 'nav-requests',
    root: '/requests',
    paths: ['/requests', '/sessions', '/logs'],
    tabs: [
      { to: '/requests', label: '路由' },
      { to: '/sessions', label: '会话' },
      { to: '/logs', label: '日志' },
    ],
  },
  { id: 'usage', label: '用量', icon: 'nav-stats', root: '/stats', paths: ['/stats'] },
  {
    id: 'gateway',
    label: '网关',
    icon: 'nav-gateway',
    root: '/gateway',
    paths: ['/gateway', '/tokens', '/diagnostics'],
    tabs: [
      { to: '/gateway', label: '概览' },
      { to: '/tokens', label: '网关密钥' },
      { to: '/diagnostics', label: '诊断' },
    ],
  },
  { id: 'settings', label: '设置', icon: 'nav-settings', root: '/settings', paths: ['/settings'] },
] as const satisfies ReadonlyArray<ModuleDef>

export type ModuleId = (typeof MODULES)[number]['id']

/** 侧栏分组：与原型一致，单组 7 项。 */
export const SIDEBAR_GROUPS: ReadonlyArray<ReadonlyArray<ModuleId>> = [
  ['apps', 'providers', 'groups', 'activity', 'usage', 'gateway', 'settings'],
]

/** 底部低频系统区（原型已取消；保留导出与空数组，避免破坏 AppLayout）。 */
export const SYSTEM_NAV: ReadonlyArray<ModuleId> = []

/** 按 id 取模块定义（nav.ts 常量表内必有）。 */
export function moduleById(id: ModuleId): (typeof MODULES)[number] {
  const found = MODULES.find((item) => item.id === id)
  return found as (typeof MODULES)[number]
}

/** 按当前路径找所属模块（命中模块 paths 中任意一项即算）。 */
export function findModule(pathname: string): (typeof MODULES)[number] | undefined {
  return MODULES.find((item) =>
    item.paths.some((path) => pathname === path || pathname.startsWith(`${path}/`)),
  )
}

/** 模块是否带二级标签页（type narrowing：union 成员 tabs 为可选）。 */
export function tabsOf(
  module: (typeof MODULES)[number] | undefined,
): readonly ModuleTab[] | undefined {
  if (module === undefined) return undefined
  return 'tabs' in module && module.tabs ? module.tabs : undefined
}
