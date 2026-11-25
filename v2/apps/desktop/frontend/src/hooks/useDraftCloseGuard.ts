/**
 * useDraftCloseGuard：全局窗口关闭守卫（product-design §8.4 D1 收尾）。
 *
 * 事实核查（2026-11-18）：store 已导出 `draftAllowsClose` / `registerDraftCloseGuard`，
 * 但全仓无任何调用方；TitleBar 的 win.close() 与 OS 关闭按钮都会静默丢草稿。
 * 本 hook 在 AppLayout 挂载一次，把守卫接到两个入口：
 *
 * - Tauri：`getCurrentWindow().onCloseRequested`。拦截时**先同步** `event.preventDefault()`
 *   （必须赶在 handler 返回前，见 @tauri-apps/api 文档），再弹确认；
 *   确认关闭后置 `confirmedClose` 标记并重新 `win.close()`——第二次 close-requested
 *   直接放行，不会递归拦截。
 * - 浏览器（dev / 非 Tauri 宿主）：`beforeunload` 原生确认框（returnValue 置空串）。
 *
 * 判定一律用 store 的统一谓词 `draftAllowsClose()`（每次关闭时实时求值，
 * 不缓存、不复制状态逻辑；从配置页切到别的页面同样生效——store 是全局的，
 * 本 hook 挂在常驻的 AppLayout）。状态语义（哪些状态算不可丢）由 store 负责。
 *
 * cleanup 安全：Tauri 监听是 Promise 返回的 UnlistenFn，用 `disposed` 标记
 * 保证"卸载先于监听建立"时也能正确退订；beforeunload 走同步 add/removeEventListener。
 */
import { useEffect } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from '../lib/tauri'
import { draftAllowsClose } from '../stores/configDraftStore'
import { flushConfigSave, waitForAutosaveIdle } from '../stores/autosaveController'
import { usePrefsStore } from '../stores/prefsStore'

export function useDraftCloseGuard() {
  const confirmOnExit = usePrefsStore((s) => s.confirmOnExit)
  const closeToTray = usePrefsStore((s) => s.closeToTray)

  useEffect(() => {
    // 关闭到托盘：关闭只隐藏窗口，草稿不会丢，不需要任何守卫。
    if (closeToTray && isTauri()) return
    // 桌面壳里「退出前询问」由 ExitGuard 统一处理（其确认框会同时提示脏草稿）：
    // 两套守卫同时注册会弹两次确认框，而且草稿守卫的 close() 会被壳侧守卫吞掉。
    // 浏览器（dev）没有 ExitGuard，beforeunload 仍要保护草稿。
    if (confirmOnExit && isTauri()) return

    if (isTauri()) {
      let unlisten: (() => void) | null = null
      let disposed = false
      let confirmedClose = false

      void getCurrentWindow()
        .onCloseRequested((event) => {
          // 确认后的第二次 close 请求（或草稿本就干净）：放行，不拦截。
          if (confirmedClose || draftAllowsClose()) return
          // 必须在 handler 返回（await 前）同步阻止默认关闭，否则窗口直接销毁。
          event.preventDefault()
          // 自动保存：先把未保存改动写盘（连同输入框里的内容），成功就直接关闭。
          void (async () => {
            const flushed = await flushConfigSave({ force: true })
            if (!flushed) await waitForAutosaveIdle()
            if (flushed || draftAllowsClose()) {
              confirmedClose = true
              void getCurrentWindow().close()
              return
            }
            const proceed = window.confirm(
              '有未保存的配置修改（校验未通过或保存失败），关闭窗口将丢失这些修改。确定要关闭吗？',
            )
            if (proceed) {
              // 确认丢草稿：放行下一次 close-requested，避免递归拦截。
              confirmedClose = true
              void getCurrentWindow().close()
            }
          })()
        })
        .then((fn) => {
          if (disposed) fn()
          else unlisten = fn
        })
        .catch(() => {
          /* 桥不可用（理论上 isTauri 已保证）：退化为无守卫，不阻塞关闭。 */
        })

      return () => {
        disposed = true
        unlisten?.()
      }
    }

    // 非 Tauri（浏览器）：原生 beforeunload 确认框。
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      if (draftAllowsClose()) return
      event.preventDefault()
      event.returnValue = ''
    }
    window.addEventListener('beforeunload', onBeforeUnload)
    return () => window.removeEventListener('beforeunload', onBeforeUnload)
  }, [confirmOnExit, closeToTray])
}
