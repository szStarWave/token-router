import { isDevToolsEnabled } from './flowy/server'

function isBlockedDevToolsShortcut(event: KeyboardEvent): boolean {
  if (event.key === 'F12') return true
  if (!event.shiftKey) return false
  if (!event.ctrlKey && !event.metaKey) return false
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
      if (isBlockedDevToolsShortcut(event)) {
        event.preventDefault()
        event.stopPropagation()
      }
    },
    { capture: true },
  )
}
