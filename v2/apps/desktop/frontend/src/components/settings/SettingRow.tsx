/**
 * 设置行：左边是标题，右边是控件；行间有细分隔线。
 *
 * 参照 Magpie 的设置页形态：一行一个设置，控件右对齐，
 * 不用表单卡片堆叠，也不给每行套边框。
 * 说明文字不占版面：作为行标题的 title 提示，搜索仍按说明匹配。
 */
import type { ReactNode } from 'react'

export interface SettingRowProps {
  label: string
  /** 悬停说明（口径、影响范围）；搜索关键词也用它。 */
  hint?: string
  children: ReactNode
}

export function SettingRow({ label, hint, children }: SettingRowProps) {
  return (
    <div className="app-setting-row" title={hint}>
      <div className="app-setting-label">
        <span className="text-ui-label text-foreground">{label}</span>
      </div>
      <div className="app-setting-control">{children}</div>
    </div>
  )
}

/** 设置分组标题（分节头，不是卡片标题）。 */
export function SettingGroup({ title }: { title: string }) {
  return <h2 className="app-setting-group">{title}</h2>
}
