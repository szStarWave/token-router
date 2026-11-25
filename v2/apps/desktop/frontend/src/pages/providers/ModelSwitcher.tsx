/**
 * 模型切换器（可搜索）：输入过滤 + 键盘上下/回车选择。
 *
 * 用于「模型设置」面板头部——供应商目录可能有几十上百个模型，原生 select 只能
 * type-ahead 跳首字母。这里提供一个轻量 combobox：
 * - 聚焦时清空输入并展示全部模型（placeholder 保留当前值）；
 * - 输入按 model id / 显示名过滤；
 * - ↑/↓ 移动高亮、Enter 选中、Esc 关闭；
 * - 点击选项用 onMouseDown 阻止默认失焦，避免下拉先关掉。
 *
 * 没有引入第三方组件库；下拉样式走 `app-combobox*` shortcuts。
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import type { CatalogModel } from '../../api/types'

export function ModelSwitcher({
  models,
  value,
  onChange,
  label = '选择要配置的模型',
}: {
  models: CatalogModel[]
  value: string | null
  onChange: (modelId: string) => void
  label?: string
}) {
  const current = models.find((model) => model.model_id === value) ?? null
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [highlight, setHighlight] = useState(0)
  const inputRef = useRef<HTMLInputElement | null>(null)

  const needle = query.trim().toLowerCase()
  const filtered = useMemo(
    () =>
      needle === ''
        ? models
        : models.filter(
            (model) =>
              model.model_id.toLowerCase().includes(needle) ||
              model.display_name.toLowerCase().includes(needle),
          ),
    [models, needle],
  )

  useEffect(() => {
    if (!open) return
    setHighlight((index) => Math.min(Math.max(index, 0), Math.max(filtered.length - 1, 0)))
  }, [filtered.length, open])

  const close = () => {
    setOpen(false)
    setQuery('')
  }

  const choose = (modelId: string) => {
    onChange(modelId)
    close()
    inputRef.current?.blur()
  }

  return (
    <div className="app-combobox w-72">
      <input
        ref={inputRef}
        className="oc-field app-field-light h-8 w-full font-mono text-meta"
        value={open ? query : (current?.model_id ?? '')}
        placeholder={current?.model_id ?? '搜索模型…'}
        aria-label={label}
        role="combobox"
        aria-expanded={open}
        aria-controls="model-switcher-list"
        onFocus={() => {
          setOpen(true)
          setQuery('')
          setHighlight(
            current === null ? 0 : Math.max(models.findIndex((m) => m.model_id === current.model_id), 0),
          )
        }}
        onBlur={close}
        onChange={(event) => {
          setQuery(event.target.value)
          setOpen(true)
          setHighlight(0)
        }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') {
            event.preventDefault()
            setOpen(true)
            setHighlight((index) => Math.min(index + 1, Math.max(filtered.length - 1, 0)))
          } else if (event.key === 'ArrowUp') {
            event.preventDefault()
            setHighlight((index) => Math.max(index - 1, 0))
          } else if (event.key === 'Enter') {
            event.preventDefault()
            const picked = filtered[highlight]
            if (picked) choose(picked.model_id)
          } else if (event.key === 'Escape') {
            event.preventDefault()
            close()
            inputRef.current?.blur()
          }
        }}
      />
      {open && (
        <div className="app-combobox-list" role="listbox" id="model-switcher-list">
          {filtered.length === 0 ? (
            <p className="px-2 py-1.5 text-meta text-muted-foreground">没有匹配的模型</p>
          ) : (
            filtered.map((model, index) => (
              <button
                key={model.model_id}
                type="button"
                role="option"
                aria-selected={model.model_id === value}
                className={
                  index === highlight
                    ? 'app-combobox-option app-combobox-option-active'
                    : 'app-combobox-option'
                }
                onMouseDown={(event) => {
                  event.preventDefault()
                  choose(model.model_id)
                }}
                onMouseEnter={() => setHighlight(index)}
              >
                <span className="font-mono truncate min-w-0">{model.model_id}</span>
                {model.model_id === value ? (
                  <span className="ml-auto shrink-0 text-meta text-muted-foreground">当前</span>
                ) : model.selected ? (
                  <span className="ml-auto shrink-0 text-meta text-muted-foreground">已选</span>
                ) : null}
              </button>
            ))
          )}
        </div>
      )}
    </div>
  )
}
