/**
 * DraftStatusBar：全局配置保存状态条（供 AppLayout 挂载）。
 *
 * - saved 时返回 null（安静优先）；
 * - pending / saving / invalid / error / restart-required / conflict 各有徽标；
 * - 只读展示：补救动作（立即保存 / 重试 / 冲突出路 / 重启）在页内 ConfigSaveBar，
 *   避免两个入口打架。
 */
import {
  draftIssueCount,
  useConfigDraftStore,
  configSaveStatus,
  type ConfigSaveStatus,
} from '../../stores/configDraftStore'

const STATUS_BADGE: Record<ConfigSaveStatus, { label: string; className: string } | null> = {
  saved: null,
  pending: { label: '未保存', className: 'app-pill app-pill-warn' },
  saving: { label: '保存中', className: 'app-pill' },
  invalid: { label: '待修复', className: 'app-pill app-pill-error' },
  error: { label: '保存失败', className: 'app-pill app-pill-error' },
  conflict: { label: '冲突', className: 'app-pill app-pill-error' },
  'restart-required': { label: '待重启', className: 'app-pill app-pill-warn' },
}

/** 全局状态条；AppLayout 在页头下方挂一次。无待办时返回 null。 */
export function DraftStatusBar() {
  // 分字段订阅：避免每次编辑都让常驻状态条跟着整库重渲染。
  const status = useConfigDraftStore(configSaveStatus)
  const draft = useConfigDraftStore((s) => s.draft)
  const baseRevision = useConfigDraftStore((s) => s.baseRevision)
  const conflict = useConfigDraftStore((s) => s.conflict)
  const issueCount = useConfigDraftStore(draftIssueCount)
  const badge = STATUS_BADGE[status]
  if (!draft || !badge) return null

  return (
    <div className="flex items-center gap-2 px-4 py-1 text-meta text-muted-foreground">
      <span className={badge.className}>{badge.label}</span>
      {status === 'pending' && <span>自动保存中</span>}
      {status === 'invalid' && <span>{issueCount} 个问题待修复（修复后自动保存）</span>}
      {status === 'conflict' && <span className="text-status-error">{conflict?.message}</span>}
      {status === 'restart-required' && <span>监听/鉴权变化，重启网关后生效</span>}
      {baseRevision && status !== 'conflict' && (
        <span className="font-mono">基准 {baseRevision.slice(0, 8)}…</span>
      )}
    </div>
  )
}
