/** 窗口控制（Tauri window API；浏览器环境隐藏）。 */
import { useCallback, useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from '../../lib/tauri'

export function TitleBar() {
  const [maximized, setMaximized] = useState(false)

  const refreshMax = useCallback(async () => {
    if (!isTauri()) return
    try {
      const win = getCurrentWindow()
      setMaximized(await win.isMaximized())
    } catch {
      /* ignore */
    }
  }, [])

  useEffect(() => {
    void refreshMax()
  }, [refreshMax])

  if (!isTauri()) return null

  const win = getCurrentWindow()
  return (
    <div className="flex items-center gap-0.5">
      <button
        type="button"
        className="app-win-btn"
        aria-label="最小化"
        onClick={() => void win.minimize()}
      >
        –
      </button>
      <button
        type="button"
        className="app-win-btn"
        aria-label={maximized ? '还原' : '最大化'}
        onClick={() => {
          void win.toggleMaximize().then(refreshMax)
        }}
      >
        {maximized ? '❐' : '□'}
      </button>
      <button
        type="button"
        className="app-win-btn app-win-close"
        aria-label="关闭"
        onClick={() => void win.close()}
      >
        ✕
      </button>
    </div>
  )
}
