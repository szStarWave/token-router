import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { AccountInfo, ProviderInfo } from '../../api/types'
import { emptyCandidate, emptyVirtualModel, type ConfigDraft } from '../../lib/config-edit'
import {
  groupAccountSummaries,
  groupMemberViews,
  groupModeLabel,
  groupModelChips,
  groupTitle,
  judgeSelectableCandidates,
  profileImpact,
  strategyLabels,
} from './group-view'

const provider = (id: string, displayName = id) =>
  ({ id, display_name: displayName }) as ProviderInfo

const account = (id: string, label: string, enabled = true) =>
  ({
    id,
    label,
    fingerprint: id.slice(-4),
    provider_id: 'siliconflow',
    enabled,
  }) as AccountInfo

const candidate = (id: string, model: string, enabled = true) => ({
  ...emptyCandidate(),
  id,
  model,
  enabled,
})

const group = (pool: string[]) => ({ ...emptyVirtualModel(), id: 'auto', name: 'Auto', candidate_pool: pool })

test('groupTitle 回落到 id，空组显示未命名', () => {
  assert.equal(groupTitle({ ...emptyVirtualModel(), id: 'g', name: '名字' }), '名字')
  assert.equal(groupTitle({ ...emptyVirtualModel(), id: 'g', name: '' }), 'g')
  assert.equal(groupTitle({ ...emptyVirtualModel(), id: '', name: '' }), '未命名组')
})

test('groupModeLabel 映射三种 profile 模式', () => {
  assert.equal(groupModeLabel('single'), '单次尝试')
  assert.equal(groupModeLabel('cascade'), '级联')
  assert.equal(groupModeLabel('verify_after'), '校验后升级')
  assert.equal(groupModeLabel(''), '未设置')
  assert.equal(groupModeLabel('custom'), 'custom')
})

test('strategyLabels 空配置展示 pipeline 默认集合', () => {
  assert.deepEqual(strategyLabels([]), [
    '基础排序',
    '故障转移',
    '能力升级',
    '难度分类',
    '到期前用完',
  ])
  assert.deepEqual(strategyLabels(['fast', 'unknown']), ['速度优先', 'unknown'])
})

test('profileImpact 列出共用 profile 的组', () => {
  const draft = {
    virtualModels: [
      { ...emptyVirtualModel(), id: 'a', name: 'A', profile: 'default' },
      { ...emptyVirtualModel(), id: 'b', name: 'B', profile: 'default' },
      { ...emptyVirtualModel(), id: 'c', name: 'C', profile: 'other' },
    ],
  } as ConfigDraft
  assert.deepEqual(profileImpact(draft, 'default'), ['A', 'B'])
  assert.deepEqual(profileImpact(draft, 'missing'), [])
})

test('groupMemberViews 反解供应商与账号，并标记手写/缺失候选', () => {
  const providers = [provider('siliconflow', '硅基流动')]
  const accounts = [account('1c72fadb-0000-0000-0000-000000000000', '主账号')]
  const candidates = [
    candidate('siliconflow-pro-deepseek-1c72fadb', 'pro-deepseek'),
    candidate('local-fast', 'qwen'),
  ]
  const views = groupMemberViews(
    group(['siliconflow-pro-deepseek-1c72fadb', 'local-fast', 'gone']),
    candidates,
    providers,
    accounts,
  )
  assert.equal(views[0].provider?.display_name, '硅基流动')
  assert.equal(views[0].accountText, '主账号 · …0000')
  assert.equal(views[0].manual, false)
  assert.equal(views[1].manual, true)
  assert.equal(views[1].accountText, '')
  assert.equal(views[2].missing, true)
})

test('groupModelChips 合并同一模型的多个账号', () => {
  const providers = [provider('siliconflow')]
  const accounts = [
    account('aaaaaaaa-0000-0000-0000-000000000000', 'A'),
    account('bbbbbbbb-0000-0000-0000-000000000000', 'B'),
  ]
  const candidates = [
    candidate('siliconflow-m-aaaaaaaa', 'm'),
    candidate('siliconflow-m-bbbbbbbb', 'm'),
  ]
  const chips = groupModelChips(
    groupMemberViews(group(['siliconflow-m-aaaaaaaa', 'siliconflow-m-bbbbbbbb']), candidates, providers, accounts),
  )
  assert.equal(chips.length, 1)
  assert.equal(chips[0].label, 'm')
  assert.equal(chips[0].count, 2)
})

test('judgeSelectableCandidates 接受任意启用候选，不要求预设用途', () => {
  const candidates = [
    candidate('siliconflow-deepseek-1c72fadb', 'deepseek'),
    { ...candidate('siliconflow-disabled-1c72fadb', 'disabled-model'), enabled: false },
    { ...candidate('  ', 'blank') },
  ]
  const selected = judgeSelectableCandidates(candidates)
  assert.deepEqual(
    selected.map((item) => item.id),
    ['siliconflow-deepseek-1c72fadb'],
  )
})

test('groupAccountSummaries 覆盖全部启用账号时显示任一账号', () => {
  const providers = [provider('siliconflow', '硅基流动')]
  const accounts = [
    account('aaaaaaaa-0000-0000-0000-000000000000', 'A'),
    account('bbbbbbbb-0000-0000-0000-000000000000', 'B'),
  ]
  const candidates = [
    candidate('siliconflow-m-aaaaaaaa', 'm'),
    candidate('siliconflow-m-bbbbbbbb', 'm'),
  ]
  const summaries = groupAccountSummaries(
    groupMemberViews(group(['siliconflow-m-aaaaaaaa', 'siliconflow-m-bbbbbbbb']), candidates, providers, accounts),
    providers,
    accounts,
  )
  assert.equal(summaries.length, 1)
  assert.equal(summaries[0].allAccounts, true)
  assert.match(summaries[0].detail, /全部 2 个启用账号/)
})
