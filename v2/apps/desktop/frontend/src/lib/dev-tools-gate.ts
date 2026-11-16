import { isDevToolsEnabled } from './flowy/server'

function isBlockedBrowserShortcut(event: KeyboardEvent): boolean {
  if (event.key === 'F12' || event.key === 'F5') return true

  const hasCtrlOrMeta = event.ctrlKey || event.metaKey
  if (hasCtrlOrMeta && (event.key === 'r' || event.key === 'R')) return true

  if (!event.shiftKey) return false
  if (!hasCtrlOrMeta) return false
  const key = event.key.toUpperCase()
  return key === 'I' || key === 'J' || key === 'C'
}

export function installDevToolsGate(): void {
  if (isDevToolsEnabled()) return

  document.addEventListener(
    'contextmenu',
    (event) => {
      event.preventDefault()
    },
    { capture: true },
  )

  document.addEventListener(
    'keydown',
    (event) => {
      if (isBlockedBrowserShortcut(event)) {
        event.preventDefault()
        event.stopPropagation()
      }
    },
    { capture: true },
  )
}
