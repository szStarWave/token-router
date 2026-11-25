/**
 * 路由组展开编辑器（Magpie 式单列表单）。
 *
 * 只提供 Token Router 真实生效的字段：名称/ID、成员模型、规则、路由判断、
 * 上下文窗口（最小/最大/自定义）、模型列表、推理档位（关闭思考/成员共有/自定）；模式与策略来自共享 profile，
 * 只读展示。Magpie 的组级限流、
 * 首字超时、全员忙、会话保持、压缩、脱敏、暴露名单没有对应运行语义，不渲染假开关。
 *
 * 所有写入都通过父级的 `onUpdate`（全局配置草稿），由自动保存引擎提交。
 */
import { useState, type ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { Icon } from '../../components/common/Icon'
import type { AccountInfo, ProviderInfo } from '../../api/types'
import type {
  CandidateDraft,
  ConfigDraft,
  GroupJudgeMode,
  GroupRuleDraft,
  VirtualModelDraft,
} from '../../lib/config-edit'
import { systemOneEndpoint } from '../../lib/config-edit'
import {
  candidateUnavailableReason,
  groupAccountSummaries,
  groupMemberViews,
  groupTitle,
  judgeAvailability,
  nextRuleId,
  profileImpact,
  ruleTargetProblem,
  effectiveStrategyIds,
  strategyLabel,
} from './group-view'
import { providerOfCandidate } from '../../lib/candidate-source'

interface RuleFormState {
  mode: 'add' | 'edit'
  ruleIndex: number | null
  intent: string
  prefer: string
}

/** 上下文窗口提示来源（映射配置里的 `context_window_mode`）。 */
const WINDOW_MODES = [
  { value: 'min', label: '最小成员' },
  { value: 'max', label: '最大成员' },
  { value: 'custom', label: '自定义' },
] as const

/** 高级区统一行：左侧固定标签列，控件与说明在右侧对齐。 */
function AdvancedRow({
  label,
  hint,
  children,
}: {
  label: string
  hint?: ReactNode
  children: ReactNode
}) {
  return (
    <div className="flex items-start gap-3">
      <span className="w-28 shrink-0 pt-1.5 text-meta text-muted-foreground">{label}</span>
      <div className="min-w-0 flex flex-1 flex-col gap-1.5">
        {children}
        {hint !== undefined && (
          <span className="text-micro text-muted-foreground leading-relaxed">{hint}</span>
        )}
      </div>
    </div>
  )
}

const MODE_OPTIONS = [
  { value: 'single', label: '单次尝试' },
  { value: 'cascade', label: '级联' },
  { value: 'verify_after', label: '校验后升级' },
] as const

/** 已知策略（含默认集合之外的 fast），只读展示。 */
const STRATEGY_OPTIONS = [
  'baseline',
  'failover',
  'escalation',
  'classify',
  'quota-reset',
  'fast',
] as const

const THINKING_OPTIONS = [
  { value: 'low', label: '低' },
  { value: 'high', label: '高' },
  { value: 'max', label: '最高' },
] as const

export interface GroupEditorProps {
  group: VirtualModelDraft
  index: number
  draft: ConfigDraft
  providers: ProviderInfo[]
  accounts: AccountInfo[]
  candidates: CandidateDraft[]
  judgeCandidates: CandidateDraft[]
  isDefault: boolean
  issueText: (path: string) => string | null
  onUpdate: (patch: Partial<VirtualModelDraft>) => void
  onSetDefault: () => void
  onRemoveMember: (candidateId: string) => void
  onOpenPicker: () => void
  onDelete: () => void
}

export function GroupEditor({
  group,
  index,
  draft,
  providers,
  accounts,
  candidates,
  judgeCandidates,
  isDefault,
  issueText,
  onUpdate,
  onSetDefault,
  onRemoveMember,
  onOpenPicker,
  onDelete,
}: GroupEditorProps) {
  const [ruleForm, setRuleForm] = useState<RuleFormState | null>(null)

  const members = groupMemberViews(group, candidates, providers, accounts)
  const accountSummaries = groupAccountSummaries(members, providers, accounts)
  const unavailableMembers = members.filter(
    (member) => candidateUnavailableReason(member.candidate) !== '',
  ).length
  const judge = judgeAvailability(group, candidates)
  // 「成员共有」口径：窗口与推理档位的实际能力来自成员候选。
  const memberWindows = members
    .map((member) => member.candidate?.window)
    .filter((window): window is number => typeof window === 'number' && window > 0)
  const minWindow = memberWindows.length > 0 ? Math.min(...memberWindows) : null
  const maxWindow = memberWindows.length > 0 ? Math.max(...memberWindows) : null
  const memberLevels = [
    ...new Set(members.flatMap((member) => member.candidate?.thinking_levels ?? [])),
  ]
  // 优先按 service_id（选中时写入候选 id）认回，旧配置回落到 endpoint+model。
  const matchedJudge =
    candidates.find(
      (candidate) =>
        candidate.id.trim() !== '' && candidate.id === group.judge.service.service_id,
    ) ??
    candidates.find(
      (candidate) =>
        candidate.id.trim() !== '' &&
        candidate.model === group.judge.service.model &&
        (candidate.base_url === group.judge.service.endpoint ||
          (candidate.caps.includes('judge:jev') &&
            systemOneEndpoint(candidate.base_url) === group.judge.service.endpoint)),
    )
  const brokenRules = group.rules.filter(
    (rule) => ruleTargetProblem(rule, group.candidate_pool, candidates) !== '',
  ).length
  // 决策模型（judge:jev）必须以 SystemOne 调用；用途改成决策模型后、
  // 组里还留着旧的对话 API 配置时提示修正，避免继续拿 /chat/completions 打它。
  const judgeKindMismatch =
    group.judge.mode === 'active' &&
    matchedJudge?.caps.includes('judge:jev') === true &&
    (group.judge.service.kind !== 'jev' ||
      group.judge.service.endpoint !== matchedJudge.base_url)
  const profileMode = draft.profiles[group.profile]?.mode
  const activeStrategies = new Set(effectiveStrategyIds(group.strategies))
  const profileGroups = profileImpact(draft, group.profile)
  const manualMembers = members.some((member) => member.manual)
  const providerNameOf = (candidateId: string) =>
    providerOfCandidate(providers, candidateId)?.display_name ?? ''

  const updateJudge = (patch: Partial<VirtualModelDraft['judge']>) => {
    onUpdate({ judge: { ...group.judge, ...patch }, judgeConfigured: true })
  }

  /** 选中任意供应商编译模型充当判断模型，预填隐藏的服务字段。 */
  const chooseJudgeCandidate = (candidateId: string) => {
    const candidate = candidates.find((item) => item.id === candidateId)
    if (!candidate) return
    const kind = candidate.caps.includes('judge:jev') ? 'jev' : 'llm'
    updateJudge({
      mode: 'active',
      service: {
        ...group.judge.service,
        kind,
        // 决策模型（SystemOne）走完整 `/v1/systemone`，对话式判官走 base_url。
        endpoint:
          kind === 'jev' ? systemOneEndpoint(candidate.base_url) : candidate.base_url,
        model: candidate.model,
        credential: candidate.credential,
        service_id: candidate.id,
      },
    })
  }

  const updateRule = (ruleIndex: number, patch: Partial<GroupRuleDraft>) => {
    onUpdate({
      rules: group.rules.map((rule, i) => (i === ruleIndex ? { ...rule, ...patch } : rule)),
      rulesConfigured: true,
    })
  }

  const saveRule = () => {
    if (!ruleForm) return
    const intent = ruleForm.intent.trim()
    const prefer = ruleForm.prefer
    if (!intent || !prefer || !group.candidate_pool.includes(prefer)) return
    if (ruleForm.mode === 'edit' && ruleForm.ruleIndex !== null) {
      updateRule(ruleForm.ruleIndex, { intent, prefer })
    } else {
      onUpdate({
        rules: [...group.rules, { id: nextRuleId(group.rules), intent, prefer }],
        rulesConfigured: true,
      })
    }
    setRuleForm(null)
  }

  const removeRule = (ruleIndex: number) => {
    onUpdate({
      rules: group.rules.filter((_, i) => i !== ruleIndex),
      rulesConfigured: true,
    })
    setRuleForm(null)
  }

  const toggleRuleForm = () => {
    if (ruleForm) {
      setRuleForm(null)
      return
    }
    const firstPrefer =
      group.candidate_pool.find((id) => candidates.some((item) => item.id === id)) ?? ''
    setRuleForm({ mode: 'add', ruleIndex: null, intent: '', prefer: firstPrefer })
  }

  const startEditRule = (ruleIndex: number) => {
    const rule = group.rules[ruleIndex]
    if (!rule) return
    setRuleForm({
      mode: 'edit',
      ruleIndex,
      intent: rule.intent,
      prefer: rule.prefer,
    })
  }

  const canSaveRule =
    ruleForm !== null &&
    ruleForm.intent.trim() !== '' &&
    group.candidate_pool.includes(ruleForm.prefer)

  return (
    <div className="flex flex-col gap-5">
      {/* ── 基本信息 ─────────────────────────────────────────── */}
      <div className="app-form-row">
        <label className="app-field">
          名称
          <input
            className="oc-field app-field-light min-w-52"
            value={group.name}
            aria-label={`路由组 ${index} 名称`}
            onChange={(event) => onUpdate({ name: event.target.value })}
          />
        </label>
        <label className="app-field">
          组 ID
          <input
            className="oc-field app-field-light font-mono min-w-72"
            value={group.id}
            aria-label={`路由组 ${index} id`}
            onChange={(event) => onUpdate({ id: event.target.value })}
          />
          {issueText(`virtual_models[${group.id}].id`) && (
            <span className="app-cell-error">
              {issueText(`virtual_models[${group.id}].id`)}
            </span>
          )}
        </label>
      </div>

      {/* ── 模型成员 ─────────────────────────────────────────── */}
      <section className="flex flex-col gap-2" aria-label={`路由组 ${groupTitle(group)} 模型`}>
        <div className="flex items-center gap-2 flex-wrap">
          <h3 className="text-ui-label font-semibold">模型</h3>
          {unavailableMembers > 0 && (
            <span className="app-pill app-pill-warn">{unavailableMembers} 个不可用</span>
          )}
          <button
            type="button"
            className="oc-btn-primary oc-btn-sm ml-auto"
            onClick={onOpenPicker}
          >
            <Icon name="add" />
            添加模型
          </button>
        </div>

        {manualMembers && (
          <p className="app-card-sub text-status-warning">
            有成员是手写候选，不在供应商目录中。
          </p>
        )}

        {members.length === 0 ? (
          <p className="text-meta text-muted-foreground">—</p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {members.map((member) => (
              <div
                key={member.candidateId}
                className="flex items-center gap-2.5 flex-wrap rounded-lg border border-border bg-background p-2.5"
              >
                <span className="grid size-7 shrink-0 place-items-center rounded-md bg-[color-mix(in_srgb,var(--surface-foreground)_7%,transparent)] text-muted-foreground">
                  <Icon name="nav-virtual-models" className="size-3.5" />
                </span>
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="flex items-center gap-1.5 flex-wrap">
                    <span
                      className="text-ui-label font-semibold truncate"
                      title={member.candidateId}
                    >
                      {member.candidate?.model?.trim() || member.candidateId}
                    </span>
                    {member.manual && (
                      <span
                        className="app-pill app-pill-warn"
                        title="来自配置里手写的 [[candidates]]，不在任何供应商目录中"
                      >
                        手写候选
                      </span>
                    )}
                    {member.missing && (
                      <span className="app-pill app-pill-error">候选不存在</span>
                    )}
                    {member.disabled && !member.missing && (
                      <span className="app-pill">已停用</span>
                    )}
                  </span>
                  <span className="truncate text-micro text-muted-foreground">
                    {member.provider ? member.provider.display_name : '配置手写'}
                    {member.accountText ? ` · ${member.accountText}` : ''}
                  </span>
                </span>
                <button
                  type="button"
                  className="oc-btn-default oc-btn-sm"
                  aria-label={`移除回答模型 ${member.candidateId}`}
                  onClick={() => onRemoveMember(member.candidateId)}
                >
                  <Icon name="remove" />
                  移除
                </button>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* ── 账号（只读推导） ─────────────────────────────────── */}
      <section className="flex flex-col gap-2" aria-label={`路由组 ${groupTitle(group)} 账号`}>
        <h3 className="text-ui-label font-semibold">账号</h3>
        {members.length === 0 || accountSummaries.length === 0 ? (
          <p className="text-meta text-muted-foreground">—</p>
        ) : (
          <div className="flex flex-col gap-1">
            {accountSummaries.map((summary) => (
              <div
                key={summary.provider.id}
                className="flex items-center gap-2 flex-wrap rounded-md border border-border px-2.5 py-1.5"
              >
                <span className="text-ui-label">{summary.label}</span>
                <span className="text-meta text-muted-foreground">{summary.detail}</span>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* ── 组内规则 ─────────────────────────────────────────── */}
      <section className="flex flex-col gap-2" aria-label={`路由组 ${groupTitle(group)} 规则`}>
        <div className="flex items-center gap-2 flex-wrap">
          <h3 className="text-ui-label font-semibold">规则</h3>
          <button
            type="button"
            className="oc-btn-default oc-btn-sm ml-auto"
            onClick={toggleRuleForm}
          >
            <Icon name={ruleForm ? 'collapse' : 'add'} />
            {ruleForm ? '收起表单' : '添加规则'}
          </button>
        </div>

        {group.rules.length > 0 && group.judge.mode !== 'active' && (
          <div className="app-banner app-banner-warn">
            有规则但未启用判断：先启用判断模型。
          </div>
        )}
        {group.rules.length > 0 && group.judge.mode === 'active' && !judge.ok && (
          <div className="app-banner app-banner-warn">
            判断模型不可用，规则不生效：{judge.reason}
          </div>
        )}
        {brokenRules > 0 && (
          <div className="app-banner app-banner-warn">
            优先模型不可用：{brokenRules} 条规则的优先模型已停用、删除或不在本组。
          </div>
        )}

        {group.rules.length === 0 ? null : (
          <div className="flex flex-col gap-1.5">
            {group.rules.map((rule, ruleIndex) => {
              const rulePath = `virtual_models.${index}.rules.${ruleIndex}`
              const problem = ruleTargetProblem(rule, group.candidate_pool, candidates)
              return (
                <div
                  key={ruleIndex}
                  className="flex flex-col gap-1 rounded-md border border-border p-2.5"
                >
                  <div className="flex items-start gap-2 flex-wrap">
                    <span className="flex-1 min-w-0 text-ui-label font-semibold break-words">
                      {rule.intent.trim() || '（未填写意图）'}
                    </span>
                    <div className="app-card-actions">
                      <button
                        type="button"
                        className="oc-btn-default oc-btn-sm"
                        aria-label={`编辑规则 ${rule.id}`}
                        onClick={() => startEditRule(ruleIndex)}
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        className="oc-btn-default oc-btn-sm"
                        aria-label={`删除规则 ${rule.id}`}
                        onClick={() => removeRule(ruleIndex)}
                      >
                        <Icon name="remove" />
                        删除
                      </button>
                    </div>
                  </div>
                  <div className="flex items-center gap-1.5 flex-wrap text-meta text-muted-foreground">
                    <span>命中后优先：</span>
                    <span className="font-mono">{rule.prefer || '未选择'}</span>
                    {problem && <span className="app-pill app-pill-warn">{problem}</span>}
                  </div>
                  {issueText(`${rulePath}.intent`) && (
                    <span className="app-cell-error">{issueText(`${rulePath}.intent`)}</span>
                  )}
                  {issueText(`${rulePath}.id`) && (
                    <span className="app-cell-error">{issueText(`${rulePath}.id`)}</span>
                  )}
                  {issueText(`${rulePath}.prefer`) && (
                    <span className="app-cell-error">{issueText(`${rulePath}.prefer`)}</span>
                  )}
                </div>
              )
            })}
          </div>
        )}

        {ruleForm && (
          <div className="app-plan-entry">
            <div className="app-plan-head">
              <span className="font-semibold">
                {ruleForm.mode === 'edit' ? '编辑规则' : '添加规则'}
              </span>
              {ruleForm.mode === 'edit' && ruleForm.ruleIndex !== null && (
                <span className="font-mono text-muted-foreground">
                  {group.rules[ruleForm.ruleIndex]?.id ?? ''}
                </span>
              )}
            </div>
            <div className="app-form-row">
              <label className="app-field flex-1 min-w-40">
                任务描述
                <input
                  className="oc-field app-field-light"
                  value={ruleForm.intent}
                  placeholder="如：写测试或修测试"
                  aria-label={`路由组 ${index} 规则表单意图`}
                  onChange={(event) =>
                    setRuleForm({ ...ruleForm, intent: event.target.value })
                  }
                />
              </label>
              <label className="app-field">
                优先模型（仅本组）
                <select
                  className="app-select"
                  value={ruleForm.prefer}
                  aria-label={`路由组 ${index} 规则表单优先候选`}
                  onChange={(event) =>
                    setRuleForm({ ...ruleForm, prefer: event.target.value })
                  }
                >
                  <option value="">（选择本组候选）</option>
                  {group.candidate_pool.map((candidateId) => {
                    const candidate = candidates.find((item) => item.id === candidateId)
                    const reason = candidateUnavailableReason(candidate)
                    return (
                      <option key={candidateId} value={candidateId}>
                        {candidate?.model || candidateId}
                        {reason ? `（${reason}）` : ''}
                      </option>
                    )
                  })}
                  {ruleForm.prefer !== '' &&
                    !group.candidate_pool.includes(ruleForm.prefer) && (
                      <option value={ruleForm.prefer} disabled>
                        {ruleForm.prefer}（不在本组）
                      </option>
                    )}
                </select>
              </label>
            </div>
            <div className="app-card-actions">
              <button
                type="button"
                className="oc-btn-primary oc-btn-sm"
                disabled={!canSaveRule}
                onClick={saveRule}
              >
                保存规则
              </button>
              <button
                type="button"
                className="oc-btn-default oc-btn-sm"
                onClick={() => setRuleForm(null)}
              >
                取消
              </button>
            </div>
          </div>
        )}
      </section>

      {/* ── 路由判断 ─────────────────────────────────────────── */}
      <section className="flex flex-col gap-2" aria-label={`路由组 ${groupTitle(group)} 路由判断`}>
        <div className="flex items-center gap-2 flex-wrap">
          <h3 className="text-ui-label font-semibold">路由判断</h3>
          {group.judge.mode === 'active' && !judge.ok && (
            <span className="app-pill app-pill-warn">判断不可用</span>
          )}
        </div>

        <div className="app-form-row">
          <label className="app-field">
            判断方式
            <select
              className="app-select w-40"
              value={group.judge.mode}
              aria-label={`路由组 ${index} 判断方式`}
              onChange={(event) =>
                updateJudge({ mode: event.target.value as GroupJudgeMode })
              }
            >
              <option value="off">不使用</option>
              <option value="active">启用判断</option>
            </select>
          </label>
          {group.judge.mode === 'active' && (
            <label className="app-field app-field-ctl">
              判断模型
              <select
                className="app-select w-72"
                value={matchedJudge?.id ?? ''}
                aria-label={`路由组 ${index} 判断模型`}
                onChange={(event) => chooseJudgeCandidate(event.target.value)}
              >
                <option value="">
                  {judgeCandidates.length === 0 ? '没有可用模型' : '选择模型…'}
                </option>
                {matchedJudge &&
                  !judgeCandidates.some((candidate) => candidate.id === matchedJudge.id) && (
                    <option value={matchedJudge.id}>
                      {matchedJudge.id}（当前配置，不在列表）
                    </option>
                  )}
                {judgeCandidates.map((candidate) => (
                  <option key={candidate.id} value={candidate.id} title={candidate.id}>
                    {candidate.caps.includes('judge:jev')
                      ? '[决策] '
                      : candidate.caps.includes('judge:llm')
                        ? '[判断] '
                        : ''}
                    {candidate.model || candidate.id}
                    {providerNameOf(candidate.id) ? ` · ${providerNameOf(candidate.id)}` : ''}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>

        {group.judge.mode === 'active' && matchedJudge && (
          <p className="text-meta text-muted-foreground">
            调用方式：
            {group.judge.service.kind === 'jev'
              ? `SystemOne（POST ${group.judge.service.endpoint}）`
              : `对话 API（POST ${group.judge.service.endpoint
                  .replace(/\/+$/, '')
                  .replace(/\/chat\/completions$/, '')}/chat/completions）· ${
                  group.judge.service.disable_thinking ? '思考关闭' : '思考开启'
                }`}
          </p>
        )}

        {judgeKindMismatch && matchedJudge && (
          <div className="app-banner app-banner-warn">
            <span>该模型是 SystemOne 决策模型，当前仍按对话 API 调用。</span>
            <button
              type="button"
              className="oc-btn-default oc-btn-sm"
              onClick={() => chooseJudgeCandidate(matchedJudge.id)}
            >
              改用 SystemOne
            </button>
          </div>
        )}

        {group.judge.mode === 'active' && !judge.ok && (
          <div className="app-banner app-banner-warn">
            <span>
              判断不可用：{judge.reason}。
              {judgeCandidates.length === 0 && (
                <>
                  {' '}
                  <Link to="/providers" className="underline">
                    供应商页
                  </Link>
                  编译模型后可选。
                </>
              )}
            </span>
          </div>
        )}

        {issueText(`virtual_models.${index}.judge.mode`) && (
          <span className="app-cell-error">
            {issueText(`virtual_models.${index}.judge.mode`)}
          </span>
        )}
      </section>

      {/* ── 高级（只读 + 少量真实字段） ───────────────────────── */}
      <details className="rounded-lg border border-border px-3 py-2">
        <summary className="cursor-pointer text-ui-label font-semibold">高级</summary>
        <div className="mt-3 flex flex-col gap-3">
          <AdvancedRow
            label="模式"
            hint={`profile ${group.profile} · ${profileGroups.length} 个组共用（只读）`}
          >
            <div className="oc-seg w-fit opacity-70 cursor-not-allowed" aria-disabled="true">
              {MODE_OPTIONS.map((mode) => (
                <button
                  key={mode.value}
                  type="button"
                  disabled
                  className={
                    profileMode === mode.value ? 'oc-seg-item-active' : 'oc-seg-item'
                  }
                  aria-pressed={profileMode === mode.value}
                  title="来自共享 profile，只读"
                >
                  {mode.label}
                </button>
              ))}
            </div>
          </AdvancedRow>

          <AdvancedRow label="策略" hint="来自 profile（未显式配置时为默认集合），只读">
            <div className="oc-seg w-fit opacity-70 cursor-not-allowed" aria-disabled="true">
              {STRATEGY_OPTIONS.map((id) => (
                <button
                  key={id}
                  type="button"
                  disabled
                  className={activeStrategies.has(id) ? 'oc-seg-item-active' : 'oc-seg-item'}
                  aria-pressed={activeStrategies.has(id)}
                  title="来自共享 profile，只读"
                >
                  {strategyLabel(id)}
                </button>
              ))}
            </div>
          </AdvancedRow>

          <AdvancedRow
            label="上下文窗口"
            hint={
              group.contextWindowMode === 'min'
                ? `按成员最小窗口${minWindow !== null ? `（${minWindow}）` : ''}，只作路由信号`
                : group.contextWindowMode === 'max'
                  ? `按成员最大窗口${maxWindow !== null ? `（${maxWindow}）` : ''}，只作路由信号`
                  : '显式值只作路由信号；实际请求窗口由所选成员决定'
            }
          >
            <div className="flex items-center gap-2 flex-wrap">
              <div className="oc-seg" role="group" aria-label={`路由组 ${index} 上下文窗口来源`}>
                {WINDOW_MODES.map((mode) => (
                  <button
                    key={mode.value}
                    type="button"
                    className={
                      group.contextWindowMode === mode.value
                        ? 'oc-seg-item-active'
                        : 'oc-seg-item'
                    }
                    aria-pressed={group.contextWindowMode === mode.value}
                    onClick={() =>
                      onUpdate({
                        contextWindowMode: mode.value,
                        context_window: mode.value === 'custom' ? group.context_window : null,
                      })
                    }
                  >
                    {mode.label}
                  </button>
                ))}
              </div>
              {group.contextWindowMode === 'custom' && (
                <input
                  type="number"
                  min={1}
                  className="oc-field app-field-light w-32"
                  value={group.context_window ?? ''}
                  placeholder="tokens"
                  aria-label={`路由组 ${index} 自定义上下文窗口`}
                  onChange={(event) => {
                    const value = event.target.value.trim()
                    onUpdate({
                      context_window: value === '' ? null : Math.max(1, Number(value) || 1),
                    })
                  }}
                />
              )}
            </div>
          </AdvancedRow>

          <AdvancedRow
            label="模型列表"
            hint={`开启后 /v1/models 额外列出「${group.id || '组'}/成员模型」，Agent 可直接选成员（仍按本组路由与账号）`}
          >
            <div className="oc-seg w-fit" role="group" aria-label={`路由组 ${index} 模型列表`}>
              <button
                type="button"
                className={group.listMembers ? 'oc-seg-item' : 'oc-seg-item-active'}
                aria-pressed={!group.listMembers}
                onClick={() => onUpdate({ listMembers: false })}
              >
                只列出本组
              </button>
              <button
                type="button"
                className={group.listMembers ? 'oc-seg-item-active' : 'oc-seg-item'}
                aria-pressed={group.listMembers}
                onClick={() => onUpdate({ listMembers: true })}
              >
                也列出成员模型
              </button>
            </div>
          </AdvancedRow>

          <AdvancedRow
            label="推理档位"
            hint={
              group.disableThinking
                ? '本组请求不发推理参数；硅基流动 / DashScope / 智谱 / OpenRouter 附带厂商关闭开关'
                : `成员共有：跟随成员候选声明的档位${
                    memberLevels.length > 0 ? `（${memberLevels.join(' / ')}）` : '（暂无）'
                  }；自定会写进组配置`
            }
          >
            <div className="flex items-center gap-2 flex-wrap">
              <div className="oc-seg" role="group" aria-label={`路由组 ${index} 推理档位`}>
                <button
                  type="button"
                  className={group.disableThinking ? 'oc-seg-item-active' : 'oc-seg-item'}
                  aria-pressed={group.disableThinking}
                  onClick={() => onUpdate({ disableThinking: true })}
                >
                  关闭思考
                </button>
                <button
                  type="button"
                  className={
                    !group.disableThinking && group.thinking_levels.length === 0
                      ? 'oc-seg-item-active'
                      : 'oc-seg-item'
                  }
                  aria-pressed={!group.disableThinking && group.thinking_levels.length === 0}
                  onClick={() => onUpdate({ disableThinking: false, thinking_levels: [] })}
                >
                  成员共有
                </button>
                <button
                  type="button"
                  className={
                    !group.disableThinking && group.thinking_levels.length > 0
                      ? 'oc-seg-item-active'
                      : 'oc-seg-item'
                  }
                  aria-pressed={!group.disableThinking && group.thinking_levels.length > 0}
                  onClick={() =>
                    onUpdate({
                      disableThinking: false,
                      thinking_levels:
                        group.thinking_levels.length > 0 ? group.thinking_levels : memberLevels,
                    })
                  }
                >
                  自定
                </button>
              </div>
              {!group.disableThinking &&
                group.thinking_levels.length > 0 &&
                THINKING_OPTIONS.map((level) => (
                  <label key={level.value} className="app-confirm">
                    <input
                      type="checkbox"
                      className="accent-primary"
                      checked={group.thinking_levels.includes(level.value)}
                      onChange={() =>
                        onUpdate({
                          thinking_levels: group.thinking_levels.includes(level.value)
                            ? group.thinking_levels.filter((item) => item !== level.value)
                            : [...group.thinking_levels, level.value],
                        })
                      }
                    />
                    {level.label}
                  </label>
                ))}
            </div>
          </AdvancedRow>

          <div className="flex items-center gap-2 flex-wrap pt-1">
            {isDefault ? (
              <span className="app-pill app-pill-ok">默认入口</span>
            ) : (
              <button
                type="button"
                className="oc-btn-default oc-btn-sm"
                disabled={group.candidate_pool.length === 0 || group.id.trim() === ''}
                title={
                  group.candidate_pool.length === 0
                    ? '先添加成员模型'
                    : '设为客户端未指定模型时的默认入口'
                }
                onClick={onSetDefault}
              >
                设为默认入口
              </button>
            )}
            <a
              className="oc-btn-default oc-btn-sm"
              href={`#/playground?target=${encodeURIComponent(`virtual:${group.id}`)}`}
              aria-label={`在模型测试中打开 ${group.id}`}
            >
              <Icon name="nav-playground" />
              测试本组
            </a>
            <button
              type="button"
              className="oc-btn-default oc-btn-sm ml-auto text-status-error"
              onClick={onDelete}
            >
              <Icon name="remove" />
              删除该组
            </button>
          </div>
        </div>
      </details>
    </div>
  )
}
