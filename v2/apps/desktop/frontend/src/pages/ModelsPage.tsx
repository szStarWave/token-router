/**
 * Models（§7.3）：候选（candidate）编辑器。
 * 增删改/禁用/引用检查/validate/apply。
 */
import { useEffect, useState } from 'react'
import {
  useConfigQuery,
  useConfigValidateMutation,
  useConfigApplyMutation,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { useAppStore } from '../stores/appStore'
import {
  draftFromDocument,
  draftToToml,
  referencesOf,
  localIssues,
  issuesByPath,
  emptyCandidate,
  type CandidateDraft,
} from '../lib/config-edit'

export function ModelsPage() {
  const configQuery = useConfigQuery()
  const validateMutation = useConfigValidateMutation()
  const applyMutation = useConfigApplyMutation()
  const showToast = useAppStore((s) => s.showToast)

  const [draft, setDraft] = useState<ReturnType<typeof draftFromDocument> | null>(null)
  const [serverIssues, setServerIssues] = useState<ReturnType<typeof draftFromDocument> extends never ? never : Awaited<ReturnType<typeof validateMutation.mutateAsync>>['issues']>([])
  const [validRevision, setValidRevision] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null)

  // 配置加载后初始化草稿（一次性；重新进入页面才重置）。
  useEffect(() => {
    if (configQuery.data && draft === null) {
      setDraft(draftFromDocument(configQuery.data))
    }
  }, [configQuery.data, draft])

  // revision 由 config page 头部提供（CAS）
  const local = draft ? localIssues(draft) : []
  const allIssues = [...local, ...serverIssues]
  const issueMap = issuesByPath(allIssues)
  const candidateIssue = (candidateId: string, field: string): string | null => {
    const prefix = `candidates[${candidateId}].${field}`
    const direct = issueMap.get(prefix)
    if (direct) return direct.map((issue) => issue.message).join('；')
    const generic = issueMap.get(`candidates[].${field}`)
    if (generic && (!candidateId || draft?.candidates.filter((c) => c.id === candidateId).length === 1)) {
      return generic.map((issue) => issue.message).join('；')
    }
    return null
  }

  if (configQuery.isLoading || (!draft && !configQuery.error)) {
    return (
      <div className="app-page">
        <QueryState loading={configQuery.isLoading} error={configQuery.error} onRetry={() => void configQuery.refetch()}>
          <span />
        </QueryState>
      </div>
    )
  }

  if (configQuery.error) {
    return (
      <div className="app-page">
        <QueryState loading={false} error={configQuery.error} onRetry={() => void configQuery.refetch()}>
          <span />
        </QueryState>
      </div>
    )
  }

  const update = (index: number, patch: Partial<CandidateDraft>) => {
    if (!draft) return
    setDraft({
      ...draft,
      candidates: draft.candidates.map((candidate, i) =>
        i === index ? { ...candidate, ...patch } : candidate,
      ),
    })
    setValidRevision(null)
    setServerIssues([])
  }

  const addCandidate = () => {
    if (!draft) return
    setDraft({ ...draft, candidates: [...draft.candidates, emptyCandidate()] })
    setValidRevision(null)
  }

  const removeCandidate = (index: number) => {
    if (!draft) return
    const candidate = draft.candidates[index]
    const references = referencesOf(draft, candidate.id)
    if (references.length > 0) {
      showToast(`candidate.referenced:${references.join(', ')}`, false)
      setConfirmDelete(null)
      return
    }
    setDraft({
      ...draft,
      candidates: draft.candidates.filter((_, i) => i !== index),
    })
    setConfirmDelete(null)
    setValidRevision(null)
  }

  const runValidate = async () => {
    if (!draft) return
    try {
      const response = await validateMutation.mutateAsync(draftToToml(draft))
      if (response.valid) {
        setValidRevision(response.revision ?? null)
        setServerIssues([])
        showToast('config.valid')
      } else {
        setServerIssues(response.issues)
        showToast('config.invalid', false)
      }
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  const runApply = async () => {
    if (!draft || !validRevision) return
    try {
      const response = await applyMutation.mutateAsync({
        revision: validRevision,
        documentToml: draftToToml(draft),
      })
      showToast(
        `config.applied:${response.revision.slice(0, 12)}${response.restart_required ? '|需重启生效' : ''}`,
      )
      setValidRevision(null)
      void configQuery.refetch()
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  return (
    <div className="app-page">
      <section className="app-card" aria-label="候选列表">
        <h2 className="app-card-title">候选（物理上游 + 模型）</h2>
        <p className="app-card-sub">
          凭据只填引用（<code className="app-inline-code">env:NAME</code> 或{' '}
          <code className="app-inline-code">file:path</code>）；密钥值不进配置。
          修改后先「校验」，无问题再「应用」。
        </p>

        {allIssues.length > 0 && (
          <div className="app-banner app-banner-warn">
            {allIssues.length} 个问题待解决（见下方标红字段）。
          </div>
        )}

        <QueryState loading={false} error={null}>
          <table className="app-table">
            <thead>
              <tr>
                <th>id</th>
                <th>tier</th>
                <th>strength</th>
                <th>provider</th>
                <th>base_url</th>
                <th>model</th>
                <th>window</th>
                <th>credential</th>
                <th>启用</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {draft?.candidates.map((candidate, index) => {
                const references = referencesOf(draft, candidate.id)
                return (
                  <tr
                    key={index}
                    className={candidateIssue(candidate.id, 'id') ? 'app-row-invalid' : undefined}
                  >
                    <td>
                      <input
                        className={
                          candidateIssue(candidate.id, 'id')
                            ? 'app-cell-input font-mono app-cell-input-invalid'
                            : 'app-cell-input font-mono'
                        }
                        value={candidate.id}
                        aria-label={`候选 ${index} id`}
                        aria-invalid={Boolean(candidateIssue(candidate.id, 'id'))}
                        onChange={(e) => update(index, { id: e.target.value })}
                      />
                      {candidateIssue(candidate.id, 'id') && (
                        <div className="app-cell-error">{candidateIssue(candidate.id, 'id')}</div>
                      )}
                    </td>
                    <td>
                      <input
                        className="app-cell-input"
                        value={candidate.tier}
                        aria-label={`候选 ${index} tier`}
                        onChange={(e) => update(index, { tier: e.target.value })}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        min={0}
                        max={100}
                        className={
                          candidateIssue(candidate.id, 'strength')
                            ? 'app-cell-input app-cell-input-invalid'
                            : 'app-cell-input'
                        }
                        value={candidate.strength}
                        aria-label={`候选 ${index} strength`}
                        onChange={(e) => update(index, { strength: Number(e.target.value) })}
                      />
                    </td>
                    <td>
                      <input
                        className="app-cell-input"
                        value={candidate.provider}
                        aria-label={`候选 ${index} provider`}
                        onChange={(e) => update(index, { provider: e.target.value })}
                      />
                    </td>
                    <td>
                      <input
                        className={
                          candidateIssue(candidate.id, 'base_url')
                            ? 'app-cell-input font-mono app-cell-input-invalid'
                            : 'app-cell-input font-mono'
                        }
                        value={candidate.base_url}
                        aria-label={`候选 ${index} base_url`}
                        onChange={(e) => update(index, { base_url: e.target.value })}
                      />
                      {candidateIssue(candidate.id, 'base_url') && (
                        <div className="app-cell-error">{candidateIssue(candidate.id, 'base_url')}</div>
                      )}
                    </td>
                    <td>
                      <input
                        className="app-cell-input font-mono"
                        value={candidate.model}
                        aria-label={`候选 ${index} model`}
                        onChange={(e) => update(index, { model: e.target.value })}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        className="app-cell-input"
                        value={candidate.window}
                        aria-label={`候选 ${index} window`}
                        onChange={(e) => update(index, { window: Number(e.target.value) })}
                      />
                    </td>
                    <td>
                      <input
                        className={
                          candidateIssue(candidate.id, 'credential')
                            ? 'app-cell-input font-mono app-cell-input-invalid'
                            : 'app-cell-input font-mono'
                        }
                        value={candidate.credential}
                        placeholder="env:NAME"
                        aria-label={`候选 ${index} credential`}
                        onChange={(e) => update(index, { credential: e.target.value })}
                      />
                      {candidateIssue(candidate.id, 'credential') && (
                        <div className="app-cell-error">
                          {candidateIssue(candidate.id, 'credential')}
                        </div>
                      )}
                    </td>
                    <td>
                      <input
                        type="checkbox"
                        className="accent-primary"
                        checked={candidate.enabled}
                        aria-label={`候选 ${index} 启用`}
                        onChange={(e) => update(index, { enabled: e.target.checked })}
                      />
                    </td>
                    <td>
                      {confirmDelete === candidate.id ? (
                        <span className="app-confirm">
                          {references.length > 0 ? `被 ${references.join(', ')} 引用` : '删除？'}
                          <button
                            type="button"
                            className="oc-btn-destructive oc-btn-sm"
                            onClick={() => removeCandidate(index)}
                          >
                            <Icon name="approve" />
                            确认
                          </button>
                          <button
                            type="button"
                            className="oc-btn-default oc-btn-sm"
                            onClick={() => setConfirmDelete(null)}
                          >
                            取消
                          </button>
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="oc-btn-default oc-btn-sm"
                          onClick={() => setConfirmDelete(candidate.id)}
                        >
                          <Icon name="remove" />
                          删除
                        </button>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
          <button type="button" className="oc-btn-default self-start" onClick={addCandidate}>
            <Icon name="add" />
            + 新增候选
          </button>
        </QueryState>
      </section>

      <section className="app-card" aria-label="保存">
        <h2 className="app-card-title">校验并应用</h2>
        <div className="app-card-actions">
          <button
            type="button"
            className="oc-btn-default"
            disabled={local.length > 0 || validateMutation.isPending || !draft}
            onClick={() => void runValidate()}
          >
            <Icon name="approve" />
            {validateMutation.isPending ? '校验中…' : '校验'}
          </button>
          <button
            type="button"
            className="oc-btn-primary"
            disabled={!validRevision || applyMutation.isPending}
            onClick={() => void runApply()}
          >
            <Icon name="apply" />
            {applyMutation.isPending ? '应用中…' : validRevision ? '应用（CAS）' : '先校验'}
          </button>
        </div>
        {validRevision && (
          <p className="app-card-sub">
            校验通过，revision{' '}
            <code className="app-inline-code font-mono">{validRevision.slice(0, 16)}…</code>。
            应用后需重启服务生效（设置页可重启）。
          </p>
        )}
      </section>
    </div>
  )
}
