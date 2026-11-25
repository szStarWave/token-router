/**
 * 配置自动保存调度器（技术方案：docs/design/config-autosave.zh.md）。
 *
 * 职责（React 之外的单例，页面只负责改草稿）：
 * - 监听草稿变化 → 800ms 防抖提交；文本输入焦点未离开时延后到 `focusout`；
 * - 提交 = `POST /v2/admin/config/apply`（If-Match = `baseRevision`），成功后基准前移；
 * - 本地校验不过不提交（状态 invalid），修好后自动重试；
 * - 422 把服务端 issues 灌回 store；网络/5xx 指数退避重试 3 次；
 * - 409 → 取最新文档做三方合并（`mergeDraft`）：无重叠静默 rebase 重试，
 *   有重叠才进冲突态（页面展示重叠字段，用户选择保留我的 / 用最新的）。
 *
 * 为什么不在每个字段上挂 onBlur：全局 `focusout` + 焦点判定能覆盖所有输入，
 * 新页面不需要重复接线，也不会漏掉某个输入框。
 */
import { api, AdminApiError, queryKeys } from '../api/client'
import type { ConfigDocument } from '../api/types'
import { draftFromDocument, draftToToml, localIssues } from '../lib/config-edit'
import { mergeDraft } from '../lib/config-draft-merge'
import { queryClient } from '../lib/query-client'
import { useAppStore } from './appStore'
import { draftIsClean, useConfigDraftStore } from './configDraftStore'

/** 编辑合并窗口：连续输入/拖拽合成一次写盘。 */
export const SAVE_DEBOUNCE_MS = 800

/** 网络/5xx 重试间隔（次数 = 长度）。 */
const RETRY_DELAYS_MS = [1_000, 2_000, 4_000]

/** 409 自动 rebase 的最大层数（超过则交给人处理，避免无限重试）。 */
const MAX_REBASE_DEPTH = 2

let timer: ReturnType<typeof setTimeout> | null = null
let retryTimer: ReturnType<typeof setTimeout> | null = null
let inFlight = false
let installed = false
let unsubscribe: (() => void) | null = null

let lastSavedAt = 0

export function lastSavedTimestamp(): number {
  return lastSavedAt
}

function clearTimer(): void {
  if (timer !== null) {
    clearTimeout(timer)
    timer = null
  }
}

function clearRetry(): void {
  if (retryTimer !== null) {
    clearTimeout(retryTimer)
    retryTimer = null
  }
}

/** 焦点在文本类输入里时不提交（避免把 `17` 这类中间态写盘）。 */
function isTextEntryFocused(): boolean {
  if (typeof document === 'undefined') return false
  const element = document.activeElement as HTMLElement | null
  if (!element) return false
  if (element.isContentEditable) return true
  if (element instanceof HTMLTextAreaElement) return true
  if (element instanceof HTMLInputElement) {
    return !['checkbox', 'radio', 'button', 'submit', 'range', 'color', 'file'].includes(
      element.type,
    )
  }
  return false
}

/** 排程一次保存；`immediate` = 立即提交（失焦 / 结构操作）。 */
export function scheduleConfigSave(options: { immediate?: boolean } = {}): void {
  const state = useConfigDraftStore.getState()
  if (!state.draft) return
  clearRetry()
  if (options.immediate) {
    clearTimer()
    void flushConfigSave()
    return
  }
  clearTimer()
  timer = setTimeout(() => {
    timer = null
    void flushConfigSave()
  }, SAVE_DEBOUNCE_MS)
}

/**
 * 立即提交当前草稿（幂等）。
 *
 * `force`：关闭窗口 / 页面隐藏时使用——不再等待输入框失焦，直接提交当前内容。
 * 返回 true = 已保存或无需保存；false = 未提交（有校验问题 / 在途 / 冲突 / 失败）。
 */
export async function flushConfigSave(options: { force?: boolean } = {}): Promise<boolean> {
  clearTimer()
  if (inFlight) return false
  const state = useConfigDraftStore.getState()
  if (!state.draft || state.applying || state.conflict) return false
  if (draftIsClean(state.draft, state.baseDocument)) return true
  if (localIssues(state.draft).length > 0 || state.serverIssues.length > 0) {
    // 本地/服务端校验不通过：不写盘，状态由 store 的 configSaveStatus 呈现为 invalid。
    return false
  }
  if (!options.force && isTextEntryFocused()) {
    // 还在输入：等 focusout（打字过程中不把半成品写进配置）。
    // 不在这里重新排程：否则输入期间每 800ms 空转一次；focusout 会立即触发提交。
    return false
  }
  inFlight = true
  try {
    return await saveOnce(0)
  } finally {
    inFlight = false
  }
}

async function saveOnce(rebaseDepth: number): Promise<boolean> {
  const state = useConfigDraftStore.getState()
  const draft = state.draft
  if (!draft || !state.beginApply()) return false
  try {
    const response = await api.configApply(state.baseRevision ?? '', draftToToml(draft))
    useConfigDraftStore.getState().commitApply(response.revision, response.restart_required)
    lastSavedAt = Date.now()
    resetRetryCount()
    void queryClient.invalidateQueries({ queryKey: queryKeys.config() })
    void queryClient.invalidateQueries({ queryKey: queryKeys.status() })
    void queryClient.invalidateQueries({ queryKey: ['admin', 'config', 'audit'] })
    if (response.restart_required) {
      useAppStore.getState().showToast('配置已保存；监听/鉴权变化需要重启网关才生效', false)
    }
    return true
  } catch (error) {
    return await handleSaveError(error, rebaseDepth)
  }
}

async function handleSaveError(error: unknown, rebaseDepth: number): Promise<boolean> {
  const store = useConfigDraftStore.getState()
  const message = error instanceof Error ? error.message : String(error)

  if (error instanceof AdminApiError) {
    // 服务端校验失败：不写盘，issues 落状态（页面标红），用户改完自动重试。
    if (error.status === 422) {
      store.setServerIssues(error.issues ?? [{ path: '', code: 'config_invalid', message }])
      useAppStore.getState().showToast('配置未保存：服务端校验未通过', false)
      return false
    }
    // 基准过期：三方 rebase。
    if (error.status === 409) {
      store.failApply()
      return await rebaseAndRetry(rebaseDepth)
    }
  }

  store.failApply()
  // 网络 / 5xx：退避重试；期间用户再编辑会重新排程。
  const attempt = retryAttempts(error)
  if (attempt >= 0 && attempt < RETRY_DELAYS_MS.length) {
    clearRetry()
    retryTimer = setTimeout(() => {
      retryTimer = null
      void flushConfigSave()
    }, RETRY_DELAYS_MS[attempt])
    return false
  }
  store.markSaveError(message)
  useAppStore.getState().showToast(`配置保存失败：${message}`, false)
  return false
}

/**
 * 退避层级：由当前 saveError 是否为本轮重试标记推导。
 * 简单可靠的做法：用模块级计数（每次成功/手动重试清零）。
 */
let retryCount = 0
function retryAttempts(_error: unknown): number {
  const attempt = retryCount
  retryCount += 1
  return attempt
}

function resetRetryCount(): void {
  retryCount = 0
}

/** 409：取最新文档三方合并；无重叠则换基准重试，有重叠进冲突态。 */
async function rebaseAndRetry(rebaseDepth: number): Promise<boolean> {
  const state = useConfigDraftStore.getState()
  const mine = state.draft
  const base = state.baseDocument
  if (!mine) return false
  if (rebaseDepth >= MAX_REBASE_DEPTH || !base) {
    state.enterConflict(null, [], '配置持续被其他会话修改，无法自动合并，请手动处理')
    return false
  }

  let fresh: ConfigDocument
  try {
    fresh = await api.config()
  } catch {
    // 取不到最新文档：保留草稿，等下一次编辑/重试。
    useConfigDraftStore.getState().failApply()
    return false
  }
  // 先灌缓存，避免 query 重取时被 ensureDraft 当成"外部变更"再次触发。
  queryClient.setQueryData(queryKeys.config(), fresh)

  const { merged, conflicts } = mergeDraft(
    draftFromDocument(base),
    mine,
    draftFromDocument(fresh),
  )
  if (conflicts.length > 0) {
    useConfigDraftStore.getState().enterConflict(fresh.revision, conflicts)
    useAppStore.getState().showToast(`配置保存冲突：${conflicts.length} 处字段被同时修改`, false)
    return false
  }
  useConfigDraftStore.getState().rebaseTo(merged, fresh)
  // 合并后本地仍可能不合法（对方引入的问题）：交给下一轮 UI 提示。
  if (localIssues(merged).length > 0) return false
  return await saveOnce(rebaseDepth + 1)
}

/** 用户点「重试」：清掉错误标记后立即提交。 */
export async function retryConfigSave(): Promise<boolean> {
  resetRetryCount()
  useConfigDraftStore.setState({ saveError: null })
  return await flushConfigSave()
}

/** 冲突处理①：保留我的改动（重叠字段取我的值）并立即写入。 */
export async function resolveConflictKeepMine(): Promise<boolean> {
  const state = useConfigDraftStore.getState()
  const mine = state.draft
  const base = state.baseDocument
  if (!mine || !base) return false
  try {
    const fresh = await api.config()
    queryClient.setQueryData(queryKeys.config(), fresh)
    const { merged } = mergeDraft(draftFromDocument(base), mine, draftFromDocument(fresh))
    useConfigDraftStore.getState().rebaseTo(merged, fresh)
    resetRetryCount()
    return await flushConfigSave()
  } catch {
    return false
  }
}

/** 冲突处理②：用最新的（丢弃我的草稿）。 */
export async function resolveConflictTakeTheirs(): Promise<void> {
  const fresh = await api.config()
  queryClient.setQueryData(queryKeys.config(), fresh)
  useConfigDraftStore.getState().resetFromDocument(fresh)
}

/** 初始化：订阅草稿变化 + 全局焦点/可见性事件。App 挂载一次。 */
export function initConfigAutosave(): () => void {
  if (installed) return () => {}
  installed = true

  unsubscribe = useConfigDraftStore.subscribe((state, prev) => {
    if (state.draft && state.draft !== prev.draft) {
      resetRetryCount()
      scheduleConfigSave()
    }
  })

  const onFocusOut = () => {
    scheduleConfigSave({ immediate: true })
  }
  const onVisibilityChange = () => {
    if (document.visibilityState === 'hidden') void flushConfigSave({ force: true })
  }
  window.addEventListener('focusout', onFocusOut, true)
  document.addEventListener('visibilitychange', onVisibilityChange)

  return () => {
    window.removeEventListener('focusout', onFocusOut, true)
    document.removeEventListener('visibilitychange', onVisibilityChange)
    unsubscribe?.()
    unsubscribe = null
    clearTimer()
    clearRetry()
    installed = false
  }
}

// 供测试/诊断：暴露内部排程状态。
export function autosavePending(): boolean {
  return timer !== null || inFlight
}

/** 等待在途保存结束（关窗前用）；超时返回 false。 */
export async function waitForAutosaveIdle(timeoutMs = 3_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs
  while (inFlight && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  return !inFlight
}

