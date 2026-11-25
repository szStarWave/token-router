/**
 * 图标层：统一走 lucide-react，但**只从这里 import**。
 *
 * DESIGN.md §Components：图标统一 `<Icon name="check" className="size-4" />`，
 * 不直接从图标库 import，也没有 `size` prop——尺寸由 className 给，
 * 同一行并排的图标共用一个尺寸。
 */
import {
  Activity,
  AlertCircle,
  AppWindow,
  ArrowRight,
  Bot,
  Boxes,
  ChartColumn,
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  CircleCheck,
  CircleSlash,
  CircleX,
  Clock,
  Code,
  Coins,
  Copy,
  ExternalLink,
  FolderOpen,
  Inbox,
  Info,
  Key,
  Layers,
  LayoutDashboard,
  Minus,
  Monitor,
  Moon,
  Network,
  Play,
  Plug,
  Plus,
  RotateCcw,
  RotateCw,
  ScrollText,
  Search,
  Settings,
  Sparkles,
  Square,
  Stethoscope,
  Sun,
  Terminal,
  Trash2,
  Upload,
  X,
  type LucideIcon,
  Server,
  Gauge,
  FlaskConical,
} from 'lucide-react'

const ICONS = {
  // 导航
  'nav-overview': LayoutDashboard,
  'nav-gateway': Network,
  'nav-apps': AppWindow,
  'nav-providers': Server,
  'nav-quotas': Gauge,
  'nav-playground': FlaskConical,
  'nav-models': Boxes,
  'nav-virtual-models': Layers,
  'nav-requests': Activity,
  'nav-stats': ChartColumn,
  'nav-logs': ScrollText,
  'nav-onboarding': Plug,
  'nav-tokens': Key,
  'nav-diagnostics': Stethoscope,
  'nav-settings': Settings,

  // 窗口
  'window-minimize': Minus,
  'window-maximize': Square,
  'window-restore': Copy,
  'window-close': X,

  // 动作
  refresh: RotateCw,
  add: Plus,
  remove: Trash2,
  copy: Copy,
  approve: Check,
  reject: X,
  apply: Upload,
  expand: ChevronRight,
  collapse: ChevronDown,
  'open-dir': FolderOpen,
  rollback: RotateCcw,
  start: Play,
  stop: Square,
  external: ExternalLink,
  search: Search,
  next: ArrowRight,

  // 状态
  success: CircleCheck,
  error: CircleX,
  warning: AlertCircle,
  info: Info,
  empty: Inbox,
  'state-pending': Circle,
  'state-skipped': CircleSlash,
  'state-in-progress': Clock,

  // 首页指标
  cost: Coins,

  // 智能体（应用）行图标
  'agent-bot': Bot,
  'agent-code': Code,
  'agent-terminal': Terminal,
  'agent-sparkle': Sparkles,
  'agent-plug': Plug,

  // 主题
  'theme-light': Sun,
  'theme-dark': Moon,
  'theme-system': Monitor,
} satisfies Record<string, LucideIcon>

export type IconName = keyof typeof ICONS

export interface IconProps {
  name: IconName
  /** 尺寸与配色都走 className（如 `size-4 text-muted-foreground`）。 */
  className?: string
}

export function Icon({ name, className = 'size-4' }: IconProps) {
  const Component = ICONS[name]
  return <Component className={className} strokeWidth={1.75} aria-hidden="true" />
}
