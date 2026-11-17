/** 窗口控制（Tauri window API；浏览器环境隐藏）。 */
import { useCallback, useEffect, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from '../../lib/tauri'
import { Icon } from '../common/Icon'

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
        <Icon name="window-minimize" className="size-3.5" />
      </button>
      <button
        type="button"
        className="app-win-btn"
        aria-label={maximized ? '还原' : '最大化'}
        onClick={() => {
          void win.toggleMaximize().then(refreshMax)
        }}
      >
        <Icon
          name={maximized ? 'window-restore' : 'window-maximize'}
          className="size-3.5"
        />
      </button>
      <button
        type="button"
        className="app-win-btn app-win-close"
        aria-label="关闭"
        onClick={() => void win.close()}
      >
        <Icon name="window-close" className="size-3.5" />
      </button>
    </div>
  )
}
