/**
 * Virtual Models（§7.3）：虚拟模型 + profile 编辑器；只改配置完成新增。
 */
import { useEffect, useState } from 'react'
import {
  useConfigQuery,
  useConfigValidateMutation,
  useConfigApplyMutation,
} from '../queries/admin'
import { QueryState, errorMessage } from '../components/common/QueryState'
import { useAppStore } from '../stores/appStore'
import {
  draftFromDocument,
  draftToToml,
  localIssues,
  emptyVirtualModel,
  type VirtualModelDraft,
} from '../lib/config-edit'

export function VirtualModelsPage() {
  const configQuery = useConfigQuery()
  const validateMutation = useConfigValidateMutation()
  const applyMutation = useConfigApplyMutation()
  const showToast = useAppStore((s) => s.showToast)

  const [draft, setDraft] = useState<ReturnType<typeof draftFromDocument> | null>(null)
  const [serverIssues, setServerIssues] = useState<Awaited<ReturnType<typeof validateMutation.mutateAsync>>['issues']>([])
  const [validRevision, setValidRevision] = useState<string | null>(null)

  useEffect(() => {
    if (configQuery.data && draft === null) {
      setDraft(draftFromDocument(configQuery.data))
    }
  }, [configQuery.data, draft])

  const local = draft ? localIssues(draft) : []
  const allIssues = [...local, ...serverIssues]

  if (configQuery.isLoading || (!draft && !configQuery.error)) {
    return (
      <div className="page">
        <QueryState loading={configQuery.isLoading} error={configQuery.error} onRetry={() => void configQuery.refetch()}>
          <span />
        </QueryState>
      </div>
    )
  }
  if (configQuery.error) {
    return (
      <div className="page">
        <QueryState loading={false} error={configQuery.error} onRetry={() => void configQuery.refetch()}>
          <span />
        </QueryState>
      </div>
    )
  }

  const update = (index: number, patch: Partial<VirtualModelDraft>) => {
    if (!draft) return
    setDraft({
      ...draft,
      virtualModels: draft.virtualModels.map((model, i) =>
        i === index ? { ...model, ...patch } : model,
      ),
    })
    setValidRevision(null)
    setServerIssues([])
  }

  const togglePoolMember = (index: number, candidateId: string) => {
    if (!draft) return
    const model = draft.virtualModels[index]
    const pool = model.candidate_pool.includes(candidateId)
      ? model.candidate_pool.filter((id) => id !== candidateId)
      : [...model.candidate_pool, candidateId]
    update(index, { candidate_pool: pool })
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
      showToast(`config.applied:${response.revision.slice(0, 12)}`)
      setValidRevision(null)
      void configQuery.refetch()
    } catch (error) {
      showToast(errorMessage(error), false)
    }
  }

  return (
    <div className="page">
      <section className="card" aria-label="虚拟模型">
        <h2>虚拟模型（客户端看到的模型 id）</h2>
        {allIssues.length > 0 && (
          <div className="banner banner-warn">{allIssues.length} 个问题待解决。</div>
        )}
        {draft?.virtualModels.map((model, index) => (
          <div key={index} className="vm-editor">
            <div className="form-row">
              <label>
                id{' '}
                <input
                  className="mono"
                  value={model.id}
                  aria-label={`虚拟模型 ${index} id`}
                  onChange={(e) => update(index, { id: e.target.value })}
                />
              </label>
              <label>
                名称{' '}
                <input
                  value={model.name}
                  aria-label={`虚拟模型 ${index} 名称`}
                  onChange={(e) => update(index, { name: e.target.value })}
                />
              </label>
              <label>
                profile{' '}
                <select
                  value={model.profile}
                  aria-label={`虚拟模型 ${index} profile`}
                  onChange={(e) => update(index, { profile: e.target.value })}
                >
                  {Object.keys(draft.profiles).map((profileId) => (
                    <option key={profileId} value={profileId}>{profileId}</option>
                  ))}
                </select>
              </label>
            </div>
            <fieldset className="scope-fieldset">
              <legend>候选池（至少一个）</legend>
              {draft.candidates
                .filter((candidate) => candidate.id.trim())
                .map((candidate) => (
                  <label key={candidate.id} className="scope-check">
                    <input
                      type="checkbox"
                      checked={model.candidate_pool.includes(candidate.id)}
                      onChange={() => togglePoolMember(index, candidate.id)}
                    />
                    <span className="mono">{candidate.id}</span>
                    <span className="text-dim">({candidate.tier})</span>
                  </label>
                ))}
            </fieldset>
            <div className="form-row">
              <label>
                策略（逗号分隔）{' '}
                <input
                  className="mono"
                  value={model.strategies.join(', ')}
                  aria-label={`虚拟模型 ${index} 策略`}
                  onChange={(e) =>
                    update(index, {
                      strategies: e.target.value
                        .split(',')
                        .map((strategy) => strategy.trim())
                        .filter(Boolean),
                    })
                  }
                />
              </label>
            </div>
          </div>
        ))}
        <button
          type="button"
          className="btn"
          onClick={() => {
            if (!draft) return
            setDraft({ ...draft, virtualModels: [...draft.virtualModels, emptyVirtualModel()] })
            setValidRevision(null)
          }}
        >
          + 新增虚拟模型
        </button>
      </section>

      <section className="card" aria-label="profile">
        <h2>Profile（路由模式）</h2>
        {Object.entries(draft?.profiles ?? {}).map(([profileId, profile]) => (
          <div key={profileId} className="vm-editor">
            <div className="form-row">
              <label>
                <span className="mono">{profileId}</span> 模式{' '}
                <select
                  value={profile.mode}
                  aria-label={`profile ${profileId} 模式`}
                  onChange={(e) => {
                    if (!draft) return
                    setDraft({
                      ...draft,
                      profiles: {
                        ...draft.profiles,
                        [profileId]: { ...profile, mode: e.target.value },
                      },
                    })
                    setValidRevision(null)
                  }}
                >
                  <option value="single">single</option>
                  <option value="cascade">cascade</option>
                  <option value="verify_after">verify_after</option>
                </select>
              </label>
              <label>
                未知价格策略{' '}
                <select
                  value={profile.unknown_price_policy}
                  aria-label={`profile ${profileId} 未知价格策略`}
                  onChange={(e) => {
                    if (!draft) return
                    setDraft({
                      ...draft,
                      profiles: {
                        ...draft.profiles,
                        [profileId]: { ...profile, unknown_price_policy: e.target.value },
                      },
                    })
                    setValidRevision(null)
                  }}
                >
                  <option value="quality_allow">quality_allow</option>
                  <option value="deny_all">deny_all</option>
                </select>
              </label>
            </div>
          </div>
        ))}
        <p className="text-dim">strength_order 与判官等高级字段先用 TOML 导入（设置页）。</p>
      </section>

      <section className="card" aria-label="保存">
        <h2>校验并应用</h2>
        <div className="card-actions">
          <button
            type="button"
            className="btn"
            disabled={local.length > 0 || validateMutation.isPending || !draft}
            onClick={() => void runValidate()}
          >
            {validateMutation.isPending ? '校验中…' : '校验'}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!validRevision || applyMutation.isPending}
            onClick={() => void runApply()}
          >
            {applyMutation.isPending ? '应用中…' : validRevision ? '应用（CAS）' : '先校验'}
          </button>
        </div>
      </section>
    </div>
  )
}
