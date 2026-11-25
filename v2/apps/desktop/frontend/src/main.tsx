import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClientProvider } from '@tanstack/react-query'
import App from './App.tsx'
import './styles/fonts.css'
import 'virtual:uno.css'
import { applyThemePref, useAppStore, watchSystemTheme } from './stores/appStore'
import { applyStoredPrefs } from './stores/prefsStore'
import { queryClient } from './lib/query-client'
import { initConfigAutosave } from './stores/autosaveController'

// 主题：先按持久化偏好落地（深/浅两套 token 都要看）。
applyThemePref(useAppStore.getState().themePref)
watchSystemTheme()
// 桌面偏好（强调色 / 动效 / 字号 / 字体）同样在首屏落地一次。
applyStoredPrefs()
// 配置自动保存：编辑 → 防抖 → apply（页面不再有校验/应用按钮）。
initConfigAutosave()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
)
