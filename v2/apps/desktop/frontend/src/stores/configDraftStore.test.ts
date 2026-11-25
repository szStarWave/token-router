/**
 * configDraftStore 单测（自动保存版）：基准定格、编辑、apply 锁、rebase、冲突、状态聚合。
 *
 * 纯 zustand vanilla store，node 环境直接可测（无 React / 无网络）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  useConfigDraftStore,
  configSaveStatus,
  draftCloseAllowed,
  draftIssueCount,
} from './configDraftStore'
import type { ConfigDocument } from '../api/types'

function doc(
  revision: string,
  overrides: { listen?: string; candidateUrl?: string } = {},
): ConfigDocument {
  return {
    schema_version: 1,
    revision,
    editable: true,
    document: {
      schema_version: 1,
      default_virtual_model: 'auto',
      server: {
        listen: overrides.listen ?? '127.0.0.1:17621',
        auth_enabled: false,
        auth_mode: 'inbound_key',
        admin_token: '',
        inbound_key: '',
      },
      profiles: {
        default: {
          mode: 'cascade',
          stream_escalation: 'next_turn',
          strength_order: [],
          difficulty_weight_micros: 0,
          hard_signal_weight_micros: 0,
          unknown_price_policy: 'quality_allow',
        },
      },
      candidates: [
        {
          id: 'a',
          tier: 'local',
          strength: 10,
          provider: 'openai',
          base_url: overrides.candidateUrl ?? 'http://127.0.0.1:11434/v1',
          model: 'qwen',
          window: 32768,
          input: ['text'],
        },
      ],
      virtual_models: [
        {
          provider: 'token-router',
          id: 'auto',
          name: 'Auto',
          profile: 'default',
          candidate_pool: ['a'],
          strategies: ['baseline'],
        },
      ],
    },
    secrets: [],
    warnings: [],
  } as unknown as ConfigDocument
}

function reset() {
  useConfigDraftStore.getState().discardDraft()
}

test('ensureDraft：创建草稿并定格基准 revision', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const state = useConfigDraftStore.getState()
  assert.equal(state.baseRevision, 'rev-base')
  assert.ok(state.draft)
  assert.equal(configSaveStatus(state), 'saved')
  assert.equal(draftCloseAllowed(state), true)
})

test('同 revision 重取不覆盖编辑；状态为 pending', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const state = useConfigDraftStore.getState()
  assert.equal(state.draft!.virtualModels[0].name, 'Coder')
  assert.equal(configSaveStatus(state), 'pending')
  assert.equal(draftCloseAllowed(state), false)
})

test('脏草稿 + 新 revision：保留我的编辑与旧基准（不再直接冲突）', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  useConfigDraftStore.getState().ensureDraft(doc('rev-other'))
  const state = useConfigDraftStore.getState()
  assert.equal(state.draft!.virtualModels[0].name, 'Coder')
  assert.equal(state.baseRevision, 'rev-base', '基准保留，提交时由 409 触发 rebase')
  assert.equal(state.conflict, null)
  assert.equal(configSaveStatus(state), 'pending')
})

test('干净草稿 + 新 revision：跟随外部变更重建', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  useConfigDraftStore.getState().ensureDraft(doc('rev-external', { listen: '127.0.0.1:19999' }))
  const state = useConfigDraftStore.getState()
  assert.equal(state.baseRevision, 'rev-external')
  assert.equal(state.draft!.server.listen, '127.0.0.1:19999')
  assert.equal(configSaveStatus(state), 'saved')
})

test('apply 期间拒绝编辑；commitApply 更新基准且 restartPending 只随 server 变化', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  assert.equal(useConfigDraftStore.getState().beginApply(), true)
  assert.equal(configSaveStatus(useConfigDraftStore.getState()), 'saving')

  const edited = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...edited, virtualModels: edited.virtualModels.map((item) => ({ ...item, name: 'Other' })) })
  assert.equal(
    useConfigDraftStore.getState().draft!.virtualModels[0].name,
    'Coder',
    'applying 期间编辑被拒',
  )

  useConfigDraftStore.getState().commitApply('rev-next', false)
  const state = useConfigDraftStore.getState()
  assert.equal(state.baseRevision, 'rev-next')
  assert.equal(state.restartPending, false)
  assert.equal(configSaveStatus(state), 'saved')
  assert.equal(state.draft!.virtualModels[0].name, 'Coder', '已提交内容成为新基准')
})

test('commitApply：restart_required 进入待重启状态', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({
    ...draft,
    server: { ...draft.server, listen: '127.0.0.1:18888' },
  })
  useConfigDraftStore.getState().beginApply()
  useConfigDraftStore.getState().commitApply('rev-next', true)
  const state = useConfigDraftStore.getState()
  assert.equal(state.restartPending, true)
  assert.equal(configSaveStatus(state), 'restart-required')
  assert.equal(draftCloseAllowed(state), true, '已落盘，不阻塞关闭')
  useConfigDraftStore.getState().clearRestartPending()
  assert.equal(configSaveStatus(useConfigDraftStore.getState()), 'saved')
})

test('rebaseTo：换基准并保留合并后的草稿', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  useConfigDraftStore.getState().beginApply()
  useConfigDraftStore.getState().failApply()
  const theirs = doc('rev-theirs', { listen: '127.0.0.1:19999' })
  const merged = {
    ...draft,
    virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })),
    server: { ...draft.server, listen: '127.0.0.1:19999' },
  }
  useConfigDraftStore.getState().rebaseTo(merged, theirs)
  const state = useConfigDraftStore.getState()
  assert.equal(state.baseRevision, 'rev-theirs')
  assert.equal(state.draft!.virtualModels[0].name, 'Coder')
  assert.equal(configSaveStatus(state), 'pending', '合并结果仍需写回一次（引擎随后自动提交）')
})

test('本地校验问题：状态 invalid 并阻止关窗', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({
    ...draft,
    candidates: draft.candidates.map((item) => ({ ...item, base_url: 'not-a-url' })),
  })
  const state = useConfigDraftStore.getState()
  assert.equal(configSaveStatus(state), 'invalid')
  assert.ok(draftIssueCount(state) > 0)
  assert.equal(draftCloseAllowed(state), false)
})

test('409 冲突态：enterConflict 展示重叠字段，resetFromDocument 恢复', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  useConfigDraftStore.getState().enterConflict('rev-theirs', ['candidates[a].model'])
  const conflicted = useConfigDraftStore.getState()
  assert.equal(configSaveStatus(conflicted), 'conflict')
  assert.deepEqual(conflicted.conflict!.paths, ['candidates[a].model'])
  assert.equal(draftCloseAllowed(conflicted), false)

  useConfigDraftStore.getState().resetFromDocument(doc('rev-theirs'))
  const resolved = useConfigDraftStore.getState()
  assert.equal(configSaveStatus(resolved), 'saved')
  assert.equal(resolved.conflict, null)
})

test('applying 中 query 重取不解锁、不改基准', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  useConfigDraftStore.getState().beginApply()
  useConfigDraftStore.getState().ensureDraft(doc('rev-other'))
  const state = useConfigDraftStore.getState()
  assert.equal(state.applying, true)
  assert.equal(state.baseRevision, 'rev-base')
})

test('failApply / setServerIssues：解锁并转 invalid', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  useConfigDraftStore.getState().beginApply()
  useConfigDraftStore.getState().setServerIssues([
    { path: 'default_virtual_model', code: 'missing', message: 'not found' },
  ])
  const state = useConfigDraftStore.getState()
  assert.equal(state.applying, false)
  assert.equal(state.serverIssues.length, 1)
  assert.equal(configSaveStatus(state), 'invalid')
})

test('markSaveError：保存失败状态与关闭拦截', () => {
  reset()
  useConfigDraftStore.getState().ensureDraft(doc('rev-base'))
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore.getState().updateDraft({ ...draft, virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })) })
  useConfigDraftStore.getState().markSaveError('network down')
  const state = useConfigDraftStore.getState()
  assert.equal(configSaveStatus(state), 'error')
  assert.equal(draftCloseAllowed(state), false)
})

test('commitApply 后 TOML 回读的 null→undefined 不算改动（不会无限重存）', () => {
  reset()
  const base = doc('rev-base') as unknown as { document: Record<string, unknown> }
  base.document.judge = { service: { mode: 'off', endpoint: '', credential: null } }
  useConfigDraftStore.getState().ensureDraft(base as unknown as ConfigDocument)
  const draft = useConfigDraftStore.getState().draft!
  useConfigDraftStore
    .getState()
    .updateDraft({
      ...draft,
      virtualModels: draft.virtualModels.map((item) => ({ ...item, name: 'Coder' })),
    })
  useConfigDraftStore.getState().beginApply()
  useConfigDraftStore.getState().commitApply('rev-next', false)
  assert.equal(
    configSaveStatus(useConfigDraftStore.getState()),
    'saved',
    '提交后的基准重建必须与草稿等价，否则自动保存会循环',
  )
})
