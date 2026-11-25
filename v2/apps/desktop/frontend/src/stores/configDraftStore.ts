/**
 * 配置草稿全局 store（自动保存版）：模型入口 / 候选 / 设置三页共享同一份草稿。
 *
 * 与旧草稿模式的差异：
 * - **不再有"校验 / 应用"两个手动步骤**：编辑 → 防抖 → 由
 *   `stores/autosaveController.ts` 自动 `apply`（If-Match = `baseRevision`）；
 * - 状态机改为 `saved / pending / saving / invalid / error / conflict / restart-required`；
 * - 服务端 revision 变化**不再直接进冲突态**：脏草稿保留，提交时若撞 409 由自动保存
 *   做三方 rebase（无重叠静默合并，有重叠才进 conflict，展示重叠字段）；
 * - 关窗守卫仍以"内容是否与基准一致"为准（`draftCloseAllowed`）。
 *
 * 不变的部分（可靠性基线）：
 * - `baseRevision` = 基准文档 revision，只在创建 / 应用成功 / rebase 时更新，apply 的 If-Match 用它；
 * - apply 期间 `applying = true`：禁止修改草稿（update 直接拒绝），保证提交内容一致；
 * - apply 成功：用已提交内容重建基准文档，避免旧 `baseDocument` 永远 dirty。
 */
import { create } from 'zustand'
import { draftFromDocument, draftToToml, localIssues, type ConfigDraft } from '../lib/config-edit'
import { parse as parseToml } from 'smol-toml'
import { deepEqualLoose } from '../lib/config-draft-diff'
import type { ConfigDocument, ConfigIssue } from '../api/types'

/** 保存状态（layout 状态条 / 页内保存栏共用）。 */
export type ConfigSaveStatus =
  | 'saved' // 与基准一致
  | 'pending' // 有未保存修改（等待防抖提交）
  | 'saving' // apply 在途
  | 'invalid' // 本地/服务端校验不通过（不提交，等修好）
  | 'error' // 保存失败（网络/5xx，已重试）
  | 'conflict' // 与磁盘改动重叠，需要人工选择
  | 'restart-required' // 已保存，但 [server] 变化需要重启网关

export interface ConfigConflictState {
  /** 服务端当前 revision（与我的基准不同）。 */
  serverRevision: string | null
  /** 用户视角的说明。 */
  message: string
  /** 双方都改过的字段路径。 */
  paths: string[]
}

interface ConfigDraftState {
  /** 草稿；null = 尚未从服务端文档创建。 */
  draft: ConfigDraft | null
  /** 草稿创建 / 上次成功应用时的服务端 revision（apply 的 If-Match 基准）。 */
  baseRevision: string | null
  /** 与 baseRevision 对应的完整文档（diff / 改动清单 / rebase 基准）。 */
  baseDocument: ConfigDocument | null
  /** 服务端校验返回的问题（下次编辑清空）。 */
  serverIssues: ConfigIssue[]
  /** apply 进行中：锁定草稿。 */
  applying: boolean
  /** 保存失败原因（网络/5xx 重试后仍失败）。 */
  saveError: string | null
  /** 已保存但需要重启网关（`[server]` 段变化）。 */
  restartPending: boolean
  /** 基准过期冲突态。 */
  conflict: ConfigConflictState | null
}

interface ConfigDraftActions {
  /** query 数据灌入。干净草稿跟随新文档；脏草稿保留（提交时 rebase）。 */
  ensureDraft: (document: ConfigDocument) => void
  /** 编辑草稿（applying 期间拒绝）。 */
  updateDraft: (next: ConfigDraft) => void
  /** 应用开始（锁定草稿）。已有 apply 在途时返回 false。 */
  beginApply: () => boolean
  /** 应用成功：以已提交草稿内容重建基准。 */
  commitApply: (newRevision: string, restartRequired: boolean) => void
  /** 应用失败（非冲突/非校验）：解锁。 */
  failApply: () => void
  /** 服务端 422：记 issues，状态转 invalid。 */
  setServerIssues: (issues: ConfigIssue[]) => void
  /** 保存失败（重试耗尽）。 */
  markSaveError: (message: string) => void
  /** 409 rebase 成功：换成合并后的草稿与新基准。 */
  rebaseTo: (draft: ConfigDraft, document: ConfigDocument) => void
  /** 409 且重叠：进入冲突态（保留草稿供对比）。 */
  enterConflict: (serverRevision: string | null, paths: string[], message?: string) => void
  /** 放弃我的修改：清草稿回到 saved（下次 ensureDraft 用最新文档重建）。 */
  discardDraft: () => void
  /** 载入最新配置重建草稿（冲突出路 / 还原到已保存）。 */
  resetFromDocument: (document: ConfigDocument) => void
  /** 清除"待重启"提示（重启完成后调用）。 */
  clearRestartPending: () => void
}

export type ConfigDraftStore = ConfigDraftState & ConfigDraftActions

/** 以草稿内容构造基准文档（commitApply 用；TOML → JSON 保持与 GET config 同构）。 */
function documentFromDraft(draft: ConfigDraft, revision: string): ConfigDocument {
  return {
    schema_version: draft.schemaVersion,
    revision,
    editable: true,
    document: parseToml(draftToToml(draft)) as ConfigDocument['document'],
    secrets: [],
    warnings: [],
  }
}

/** 草稿相对基准是否干净（内容级比较，避免只比引用漏掉重建）。 */
export function draftIsClean(draft: ConfigDraft, base: ConfigDocument | null): boolean {
  if (!base) return false
  // 宽松比较：TOML 无法表达 null，提交后回读会有 null→undefined 的等价差异。
  return deepEqualLoose(draft, draftFromDocument(base))
}

export const useConfigDraftStore = create<ConfigDraftStore>((set, get) => ({
  draft: null,
  baseRevision: null,
  baseDocument: null,
  serverIssues: [],
  applying: false,
  saveError: null,
  restartPending: false,
  conflict: null,

  ensureDraft: (document) => {
    const state = get()
    // apply 期间 query 重取：只缓存文档，CAS 结果由 commitApply/rebaseTo 处理。
    if (state.applying) {
      set({ baseDocument: state.baseDocument ?? document })
      return
    }
    // 无草稿 → 创建。
    if (!state.draft) {
      set({
        draft: draftFromDocument(document),
        baseRevision: document.revision,
        baseDocument: document,
        serverIssues: [],
        saveError: null,
        restartPending: false,
        conflict: null,
      })
      return
    }
    // 同基准（query 重取 / SSE invalidate）→ 保留用户编辑。
    if (state.baseRevision === document.revision) return
    // 新基准 + 草稿干净 → 跟随外部变更（编译候选 / 另一窗口写回等）。
    if (draftIsClean(state.draft, state.baseDocument)) {
      set({
        draft: draftFromDocument(document),
        baseRevision: document.revision,
        baseDocument: document,
        serverIssues: [],
        saveError: null,
        restartPending: false,
        conflict: null,
      })
      return
    }
    // 新基准 + 草稿脏 → 保留我的编辑与旧基准：提交时 apply 409 → 三方 rebase。
    // （自动保存下"脏草稿 + 新 revision"是常态，不能一撞就停。）
  },

  updateDraft: (next) => {
    const state = get()
    if (!state.draft || state.applying) return // apply 期间禁止修改
    set({ draft: next, serverIssues: [], saveError: null })
  },

  beginApply: () => {
    const state = get()
    if (!state.draft || state.applying) return false
    set({ applying: true, saveError: null })
    return true
  },

  commitApply: (newRevision, restartRequired) => {
    const draft = get().draft
    if (!draft) return
    set({
      applying: false,
      restartPending: restartRequired,
      conflict: null,
      serverIssues: [],
      saveError: null,
      baseRevision: newRevision,
      // 以已提交的草稿内容重建基准（旧 baseDocument 已过期，
      // 沿用会让改动清单永远非空）。query 随后重取新文档，同 revision 幂等。
      baseDocument: documentFromDraft(draft, newRevision),
    })
  },

  failApply: () => {
    set({ applying: false })
  },

  setServerIssues: (issues) => {
    set({ applying: false, serverIssues: issues, saveError: null })
  },

  markSaveError: (message) => {
    set({ applying: false, saveError: message })
  },

  rebaseTo: (draft, document) => {
    set({
      draft,
      applying: false,
      baseRevision: document.revision,
      baseDocument: document,
      conflict: null,
      serverIssues: [],
      saveError: null,
    })
  },

  enterConflict: (serverRevision, paths, message) => {
    const state = get()
    const mine = state.baseRevision?.slice(0, 8) ?? '未知'
    const theirs = serverRevision?.slice(0, 8) ?? '未知'
    set({
      applying: false,
      conflict: {
        serverRevision,
        paths,
        message:
          message ??
          `配置已被其他会话修改，且与你的改动重叠（${paths.slice(0, 5).join('、')}${
            paths.length > 5 ? ` 等 ${paths.length} 处` : ''
          }）：你的基准 ${mine}… → 当前 ${theirs}…`,
      },
    })
  },

  discardDraft: () => {
    set({
      draft: null,
      baseRevision: null,
      baseDocument: null,
      serverIssues: [],
      applying: false,
      saveError: null,
      restartPending: false,
      conflict: null,
    })
  },

  resetFromDocument: (document) => {
    set({
      draft: draftFromDocument(document),
      baseRevision: document.revision,
      baseDocument: document,
      serverIssues: [],
      applying: false,
      saveError: null,
      restartPending: false,
      conflict: null,
    })
  },

  clearRestartPending: () => {
    set({ restartPending: false })
  },
}))

/** 聚合保存状态（供 layout 状态条 / 页内保存栏统一渲染）。 */
export function configSaveStatus(state: ConfigDraftState): ConfigSaveStatus {
  if (state.conflict) return 'conflict'
  if (state.applying) return 'saving'
  if (!state.draft) return 'saved'
  if (state.saveError) return 'error'
  if (localIssues(state.draft).length > 0 || state.serverIssues.length > 0) return 'invalid'
  if (!draftIsClean(state.draft, state.baseDocument)) return 'pending'
  return state.restartPending ? 'restart-required' : 'saved'
}

/** 草稿本地问题数（状态条展示用）。 */
export function draftIssueCount(state: ConfigDraftState): number {
  if (!state.draft) return 0
  return localIssues(state.draft).length
}

// ---------------------------------------------------------------------------
// 关闭提醒全局注册（layout 对接）：任何页面产生未保存改动后，关闭窗口前提醒。
// ---------------------------------------------------------------------------

/** 关闭守卫回调返回 true = 允许关闭。 */
export type DraftCloseGuard = () => boolean

let closeGuard: DraftCloseGuard | null = null

/**
 * 注册窗口关闭守卫（AppLayout / 宿主层调用一次）。
 * 返回取消注册函数。
 */
export function registerDraftCloseGuard(guard: DraftCloseGuard): () => void {
  closeGuard = guard
  return () => {
    if (closeGuard === guard) closeGuard = null
  }
}

/** 供 Tauri onCloseRequested 等宿主钩子查询：有未保存草稿时返回 false。 */
export function draftAllowsClose(): boolean {
  const guard = closeGuard
  if (guard) return guard()
  return draftCloseAllowed(useConfigDraftStore.getState())
}

/**
 * 关闭允许谓词（纯函数）：
 * - 拦截：conflict / applying（未完成的事，结果未知）；
 * - 拦截：草稿与基准内容不一致（含 invalid / pending / error）；
 * - 允许：无草稿、或与基准一致（restart-required 已落盘，不阻塞关闭）。
 */
export function draftCloseAllowed(state: ConfigDraftState): boolean {
  if (state.conflict || state.applying) return false
  if (!state.draft) return true
  return draftIsClean(state.draft, state.baseDocument)
}
