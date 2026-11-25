/**
 * useConfigDraft（自动保存版）：配置编辑页的统一入口。
 *
 * 职责：
 * - 把 config query 数据灌入全局 store（干净草稿跟随新文档；脏草稿保留，提交时 rebase）；
 * - 暴露保存状态（saved / pending / saving / invalid / error / conflict / restart-required）；
 * - 动作交给 `stores/autosaveController.ts`：编辑后自动保存，另有手动 `saveNow` / 重试 /
 *   冲突出路（保留我的 / 用最新的）/ 还原到已保存。
 *
 * 页面只需要 `update(draft)`：提交时机、409 合并、422 校验都由引擎处理。
 */
import { useCallback, useEffect } from 'react'
import { useConfigQuery } from '../queries/admin'
import { errorMessage } from '../components/common/QueryState'
import { useAppStore } from '../stores/appStore'
import {
  draftIssueCount,
  useConfigDraftStore,
  configSaveStatus,
  type ConfigSaveStatus,
} from '../stores/configDraftStore'
import {
  flushConfigSave,
  resolveConflictKeepMine,
  resolveConflictTakeTheirs,
  retryConfigSave,
  scheduleConfigSave,
} from '../stores/autosaveController'
import { localIssues, type ConfigDraft } from '../lib/config-edit'
import type { ConfigDocument } from '../api/types'

export interface UseConfigDraftResult {
  draft: ConfigDraft | null
  baseRevision: string | null
  status: ConfigSaveStatus
  /** 本地校验问题（页面标红用）。 */
  localIssues: ReturnType<typeof localIssues>
  /** 服务端 422 返回的问题。 */
  serverIssues: ReturnType<typeof localIssues>
  issueCount: number
  saving: boolean
  saveError: string | null
  restartPending: boolean
  conflict: ReturnType<typeof useConfigDraftStore.getState>['conflict']
  /** 编辑草稿（自动保存会随后提交）。 */
  update: (next: ConfigDraft) => void
  /** 立即提交（不等防抖）。 */
  saveNow: () => Promise<boolean>
  /** 保存失败后手动重试。 */
  retrySave: () => Promise<boolean>
  /** 冲突出路：保留我的改动（重叠字段取我的）。 */
  keepMine: () => Promise<boolean>
  /** 冲突出路：用最新的（丢弃我的草稿）。 */
  takeTheirs: () => Promise<void>
  /** 还原到已保存（重新拉取服务端配置）。 */
  discard: () => Promise<void>
  /** 载入最新配置重建草稿；失败返回 null 且保留草稿。 */
  reloadLatest: () => Promise<ConfigDocument | null>
  /** 清除"待重启"状态（重启完成后）。 */
  clearRestartPending: () => void
  queryError: ReturnType<typeof useConfigQuery>['error']
  queryLoading: boolean
  refetch: () => void
}

export function useConfigDraft(): UseConfigDraftResult {
  const configQuery = useConfigQuery()
  const showToast = useAppStore((s) => s.showToast)

  const draft = useConfigDraftStore((s) => s.draft)
  const baseRevision = useConfigDraftStore((s) => s.baseRevision)
  const serverIssues = useConfigDraftStore((s) => s.serverIssues)
  const saveError = useConfigDraftStore((s) => s.saveError)
  const restartPending = useConfigDraftStore((s) => s.restartPending)
  const saving = useConfigDraftStore((s) => s.applying)
  const conflict = useConfigDraftStore((s) => s.conflict)
  const status = useConfigDraftStore(configSaveStatus)
  const issueCount = useConfigDraftStore(draftIssueCount)

  // 服务端文档到达后灌入 store（同基准幂等；脏草稿保留，不复位）。
  useEffect(() => {
    if (configQuery.data) {
      useConfigDraftStore.getState().ensureDraft(configQuery.data)
    }
  }, [configQuery.data])

  const update = useCallback((next: ConfigDraft) => {
    useConfigDraftStore.getState().updateDraft(next)
    // 结构操作（新建/删除/开关）走防抖提交；文本输入由引擎的 focusout 兜底。
    scheduleConfigSave()
  }, [])

  const saveNow = useCallback(async () => {
    const saved = await flushConfigSave()
    if (!saved) {
      const state = useConfigDraftStore.getState()
      if (state.saveError) showToast(`保存失败：${state.saveError}`, false)
    }
    return saved
  }, [showToast])

  const retrySave = useCallback(async () => retryConfigSave(), [])

  const keepMine = useCallback(async () => resolveConflictKeepMine(), [])

  const takeTheirs = useCallback(async () => {
    try {
      await resolveConflictTakeTheirs()
    } catch (error) {
      showToast(`载入最新配置失败：${errorMessage(error)}`, false)
    }
  }, [showToast])

  /** 还原到已保存：refetch 必须成功且有 data 才 reset（失败保留草稿）。 */
  const discard = useCallback(async () => {
    try {
      const result = await configQuery.refetch()
      if (result.isSuccess && result.data) {
        useConfigDraftStore.getState().resetFromDocument(result.data)
      } else {
        showToast('刷新配置失败：已保留当前草稿', false)
      }
    } catch (error) {
      showToast(`刷新配置失败：${errorMessage(error)}（已保留当前草稿）`, false)
    }
  }, [configQuery, showToast])

  const reloadLatest = useCallback(async (): Promise<ConfigDocument | null> => {
    try {
      const result = await configQuery.refetch()
      if (result.isSuccess && result.data) {
        useConfigDraftStore.getState().resetFromDocument(result.data)
        return result.data
      }
      showToast('载入最新配置失败：已保留当前草稿', false)
      return null
    } catch (error) {
      showToast(`载入最新配置失败：${errorMessage(error)}（已保留当前草稿）`, false)
      return null
    }
  }, [configQuery, showToast])

  const clearRestartPending = useCallback(() => {
    useConfigDraftStore.getState().clearRestartPending()
  }, [])

  return {
    draft,
    baseRevision,
    status,
    localIssues: draft ? localIssues(draft) : [],
    serverIssues,
    issueCount,
    saving,
    saveError,
    restartPending,
    conflict,
    update,
    saveNow,
    retrySave,
    keepMine,
    takeTheirs,
    discard,
    reloadLatest,
    clearRestartPending,
    queryError: configQuery.error,
    queryLoading: configQuery.isLoading,
    refetch: () => void configQuery.refetch(),
  }
}
