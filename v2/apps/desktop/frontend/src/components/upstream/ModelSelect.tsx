import { useEffect, useRef, useState } from 'react'

export interface ModelOption {
  id: string
  name: string
  icon?: string
}

function escapeHtml(text: string) {
  return String(text)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

function isIconUrl(icon: string) {
  const t = icon.trim()
  return t.startsWith('http://') || t.startsWith('https://') || t.startsWith('/')
}

function ModelIcon({ icon, className = 'model-select-icon' }: { icon?: string; className?: string }) {
  if (!icon?.trim()) {
    return <span className={`${className} model-select-icon-fallback`} aria-hidden="true">◇</span>
  }
  if (isIconUrl(icon)) {
    return <img className={className} src={icon} alt="" />
  }
  return (
    <span className={`${className} model-select-icon-emoji`} aria-hidden="true" dangerouslySetInnerHTML={{ __html: escapeHtml(icon) }} />
  )
}

export function resolveDefaultModelId(models: ModelOption[], preferred?: string | null) {
  const pref = preferred === 'Auto' ? 'auto' : preferred
  if (pref && models.some((m) => m.id === pref)) return pref
  const auto = models.find((m) => m.id === 'auto' || m.id === 'Auto')
  if (auto) return auto.id
  return models[0]?.id || 'auto'
}

interface ModelSelectProps {
  models: ModelOption[]
  value: string
  placeholder?: string
  onChange?: (value: string) => void
  id?: string
}

export function ModelSelect({ models, value, placeholder = 'Select model', onChange, id }: ModelSelectProps) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)

  const selected = models.find((m) => m.id === value)

  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onMouseDown)
    return () => document.removeEventListener('mousedown', onMouseDown)
  }, [])

  return (
    <div
      ref={rootRef}
      id={id}
      className={`model-select${open ? ' open' : ''}`}
    >
      <button
        type="button"
        className="model-select-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={(e) => {
          e.stopPropagation()
          setOpen(!open)
        }}
      >
        <span className="model-select-trigger-inner">
          {selected ? (
            <>
              <ModelIcon icon={selected.icon} />
              <span className="model-select-label">{selected.name || selected.id}</span>
            </>
          ) : (
            <span className="model-select-label">{placeholder}</span>
          )}
        </span>
        <svg className="model-select-chevron" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" strokeWidth="2" />
        </svg>
      </button>
      <div className="model-select-menu" role="listbox" hidden={!open}>
        {!models.length ? (
          <div className="model-select-empty">{placeholder}</div>
        ) : (
          models.map((m) => {
            const label = m.name || m.id
            const active = m.id === value
            return (
              <button
                key={m.id}
                type="button"
                className={`model-select-option${active ? ' active' : ''}`}
                role="option"
                aria-selected={active}
                onClick={() => {
                  onChange?.(m.id)
                  setOpen(false)
                }}
              >
                <ModelIcon icon={m.icon} className="model-select-option-icon" />
                <span className="model-select-option-label">{label}</span>
              </button>
            )
          })
        )}
      </div>
    </div>
  )
}
