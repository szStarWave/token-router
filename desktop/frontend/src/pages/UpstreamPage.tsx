import { useEffect, useMemo, useRef, useState } from 'react'
import { useParams } from '@tanstack/react-router'
import { useSaveSetupMutation } from '../queries/gateway'
import { useCloudModelsQuery } from '../queries/flowy'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { useEdgeStore } from '../stores/edgeStore'
import { useI18n } from '../hooks/useI18n'
import { ModelSelect } from '../components/upstream/ModelSelect'
import { EdgeModelListItem } from '../components/upstream/EdgeModelListItem'
import { HerdsmanSetupBanner } from '../components/upstream/HerdsmanSetupBanner'
import {
  AUTO_MODEL_ID,
  budgetFromSlider,
  buildCloudSavePayload,
  fetchCloudModels,
  getCloudBaseUrl,
  getCloudModelDisplayName,
  sliderFromCloudBudget,
  withAutoModelOption,
} from '../lib/cloud-upstream'
import {
  buildDisplayItems,
  deleteManualEntry,
  getEdgeModelDisplayName,
  isEdgeUpstreamConfigured,
  persistEdgeSelection,
  selectEdgeModel,
  upsertManualEntry,
  type ManualEdgeEntry,
} from '../lib/edge-upstream'
import { fmtNum, fmtPct } from '../lib/stats-utils'
import type { UpstreamSetupView } from '../types/gateway'
import { DEFAULT_CLOUD_TOKEN_BUDGET, CLOUD_BUDGET_MIN, CLOUD_BUDGET_MAX } from '../constants/defaults'
import { formatCompactNum } from '../lib/format-number'

function cloudQuotaFromSetup(cloud: UpstreamSetupView['cloud']) {
  const budget = cloud?.token_budget ?? 0
  const enabled = cloud?.token_quota_enabled ?? budget > 0
  return { budget, enabled }
}

export function UpstreamPage() {
  const { navId = 'edge' } = useParams({ strict: false })
  const isEdge = navId === 'edge'
  const { t, locale } = useI18n()
  const stats = useAppStore((s) => s.stats)
  const connected = useAppStore((s) => s.connected)
  const setup = useSetupStore((s) => s.setup)
  const herdsmanConnected = useEdgeStore((s) => s.herdsmanConnected)
  const herdsmanInstalled = useEdgeStore((s) => s.herdsmanInstalled)
  const manualEntries = useEdgeStore((s) => s.manualEntries)
  const selectedKey = useEdgeStore((s) => s.selectedKey)
  const cachedModels = useEdgeStore((s) => s.cachedModels)
  const saveSetup = useSaveSetupMutation()
  const modelsQuery = useCloudModelsQuery()

  const edgeConfigured = useMemo(
    () => isEdgeUpstreamConfigured(setup?.edge),
    [setup?.edge, herdsmanConnected, selectedKey, cachedModels],
  )

  const autoLabel = t('cloudModel.auto')
  const cloudModels = useMemo(
    () => withAutoModelOption(modelsQuery.data ?? [], autoLabel),
    [modelsQuery.data, autoLabel],
  )

  const initialCloudQuota = cloudQuotaFromSetup(setup?.cloud)
  const [cloudModel, setCloudModel] = useState(setup?.cloud?.model ?? AUTO_MODEL_ID)
  const [quotaEnabled, setQuotaEnabled] = useState(initialCloudQuota.enabled)
  const [budgetSlider, setBudgetSlider] = useState(() =>
    sliderFromCloudBudget(initialCloudQuota.budget > 0 ? initialCloudQuota.budget : DEFAULT_CLOUD_TOKEN_BUDGET),
  )
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingEntry, setEditingEntry] = useState<ManualEdgeEntry | null>(null)
  const [dialogForm, setDialogForm] = useState({ name: '', url: '', model: '', key: '', context_window: '' })
  const quotaEditingRef = useRef(false)
  const cloudSaveGenRef = useRef(0)

  useEffect(() => {
    if (quotaEditingRef.current) return
    setCloudModel(setup?.cloud?.model ?? AUTO_MODEL_ID)
    const { budget, enabled } = cloudQuotaFromSetup(setup?.cloud)
    setQuotaEnabled(enabled)
    if (budget > 0) {
      setBudgetSlider(sliderFromCloudBudget(budget))
    }
  }, [setup?.cloud])

  useEffect(() => {
    if (!isEdge && connected) void fetchCloudModels(autoLabel).catch(() => {})
  }, [isEdge, connected, autoLabel])

  const edgePct = stats?.routing?.edge_pct
  const cloudPct = stats?.routing?.cloud_pct
  const edgeReq = stats?.routing?.edge
  const cloudReq = stats?.routing?.cloud

  const displayItems = buildDisplayItems()
  const herdsmanItems = displayItems.filter((i) => i.type === 'herdsman')
  const customItems = displayItems.filter((i) => i.type === 'manual')

  const applyCloudQuotaFromSetup = (cloud: UpstreamSetupView['cloud']) => {
    const { budget, enabled } = cloudQuotaFromSetup(cloud)
    setQuotaEnabled(enabled)
    if (budget > 0) {
      setBudgetSlider(sliderFromCloudBudget(budget))
    }
  }

  const saveCloud = (model: string, slider: number, quota: boolean) => {
    if (!connected) return
    quotaEditingRef.current = true
    const saveGen = ++cloudSaveGenRef.current
    const budget = quota ? budgetFromSlider(slider) : 0
    const cloud = buildCloudSavePayload(model, budget)
    saveSetup.mutate({ cloud }, {
      onSuccess: (res) => {
        if (saveGen !== cloudSaveGenRef.current) return
        applyCloudQuotaFromSetup(res.upstream?.cloud)
        quotaEditingRef.current = false
      },
      onError: () => {
        if (saveGen === cloudSaveGenRef.current) quotaEditingRef.current = false
      },
    })
  }

  const saveEdge = () => {
    void persistEdgeSelection((body) => saveSetup.mutate(body))
  }

  const openDialog = (entry?: ManualEdgeEntry) => {
    setEditingEntry(entry ?? null)
    setDialogForm({
      name: entry?.name ?? '',
      url: entry?.base_url ?? '',
      model: entry?.model ?? '',
      key: '',
      context_window: entry?.context_window != null ? String(entry.context_window) : '',
    })
    setDialogOpen(true)
  }

  const saveDialog = () => {
    const id = editingEntry?.id ?? `manual-${Date.now()}`
    const contextRaw = dialogForm.context_window.trim()
    upsertManualEntry({
      id,
      name: dialogForm.name.trim() || dialogForm.model.trim(),
      base_url: dialogForm.url.trim(),
      model: dialogForm.model.trim(),
      api_key: dialogForm.key.trim() || editingEntry?.api_key,
      context_window: contextRaw ? Number(contextRaw) : undefined,
    })
    setDialogOpen(false)
    saveEdge()
  }

  const budgetValue = budgetFromSlider(budgetSlider)
  const edgeModelLabel = useMemo(
    () => getEdgeModelDisplayName(setup?.edge),
    [setup?.edge, herdsmanConnected, selectedKey, cachedModels],
  )

  return (
    <section className="page active" id="page-upstream">
      <div id="upstream-edge-view" className={`upstream-view${isEdge ? ' active' : ''}`}>
        <div className="upstream-hero edge">
          <div className="upstream-hero-icon">
            <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 20h8" /><path d="M12 16v4" /></svg>
          </div>
          <div className="upstream-hero-main">
            <h2>{t('upstream.edgeTitle')}</h2>
            <p>{t('upstream.edgeDesc')}</p>
          </div>
          <span className={`tag bordered ${herdsmanConnected ? 'ok' : herdsmanInstalled ? 'warn' : 'warn'} upstream-hero-status`} id="herdsman-status">
            <span className="dot" />
            <span data-herdsman-label>
              {herdsmanConnected ? t('herdsman.connected') : herdsmanInstalled ? t('herdsman.installedNotRunning') : t('herdsman.notFound')}
            </span>
          </span>
          <span className={`tag bordered ${edgeConfigured ? 'ok' : 'off'} upstream-hero-status`} id="upstream-edge-status">
            <span className="dot" />
            <span>{edgeConfigured ? t('status.configured') : t('status.notConfigured')}</span>
          </span>
        </div>

        <div className="upstream-selected-model edge">
          <span className="upstream-selected-model-label">{t('upstream.currentModel')}</span>
          {edgeModelLabel && (
            <span className="tag bordered upstream-selected-model-tag" id="upstream-edge-selected-model-tag">{edgeModelLabel}</span>
          )}
          <span className="upstream-selected-model-value" id="upstream-edge-selected-model">{edgeModelLabel || '—'}</span>
        </div>

        <div className="upstream-stats">
          <div className="stat-box edge">
            <div className="label">{t('upstream.routeShare')}</div>
            <div className="value" id="upstream-edge-route-pct">{edgePct != null ? fmtPct(edgePct) : '—'}</div>
            <div className="sub">{t('upstream.edgeRouteSub')}</div>
          </div>
          <div className="stat-box edge">
            <div className="label">{t('upstream.routeCount')}</div>
            <div className="value" id="upstream-edge-req">{edgeReq != null ? fmtNum(edgeReq, locale) : '—'}</div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-title">{t('upstream.edgeModelList')}</div>
          <div className="edge-model-subsection">
            <div className="edge-model-subsection-title">{t('edgeModel.herdsmanListTitle')}</div>
            <div id="edge-herdsman-model-list" className="edge-model-list">
              {!herdsmanConnected ? (
                <HerdsmanSetupBanner installed={herdsmanInstalled} />
              ) : !herdsmanItems.length ? (
                <div className="edge-model-list-empty">{t('edgeModel.herdsmanEmpty')}</div>
              ) : (
                herdsmanItems.map((item) => (
                  <EdgeModelListItem
                    key={item.key}
                    item={item}
                    selected={selectedKey === item.key}
                    typeLabel={t('edgeModel.herdsman')}
                    selectLabel={item.name}
                    editLabel={t('action.edit')}
                    deleteLabel={t('action.delete')}
                    onSelect={() => { selectEdgeModel(item.key); saveEdge() }}
                  />
                ))
              )}
            </div>
          </div>
          <div className="edge-model-subsection">
            <div className="edge-model-list-header">
              <div className="edge-model-subsection-title">{t('edgeModel.customListTitle')}</div>
              <button type="button" className="btn btn-primary btn-sm" id="btn-edge-model-add" onClick={() => openDialog()}>{t('action.add')}</button>
            </div>
            <div id="edge-custom-model-list" className="edge-model-list">
              {!customItems.length ? (
                <div className="edge-model-list-empty">{t('edgeModel.customEmpty')}</div>
              ) : (
                customItems.map((item) => (
                  <EdgeModelListItem
                    key={item.key}
                    item={item}
                    selected={selectedKey === item.key}
                    typeLabel={t('edgeModel.custom')}
                    selectLabel={item.name}
                    editLabel={t('action.edit')}
                    deleteLabel={t('action.delete')}
                    onSelect={() => { selectEdgeModel(item.key); saveEdge() }}
                    onEdit={() => openDialog(manualEntries.find((e) => e.id === item.id))}
                    onDelete={() => { deleteManualEntry(item.id); saveEdge() }}
                  />
                ))
              )}
            </div>
          </div>
        </div>
      </div>

      <div id="upstream-cloud-view" className={`upstream-view${!isEdge ? ' active' : ''}`}>
        <div className="upstream-hero cloud">
          <div className="upstream-hero-icon">
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 18h11a4 4 0 010-8 5 5 0 00-9.9-1.1A4 4 0 007 18z" /></svg>
          </div>
          <div className="upstream-hero-main">
            <h2>{t('upstream.cloudTitle')}</h2>
            <p>{t('upstream.cloudDesc')}</p>
          </div>
          <span className={`tag bordered ${setup?.cloud?.configured ? 'ok' : 'off'} upstream-hero-status`} id="upstream-cloud-status">
            <span className="dot" />
            <span>{setup?.cloud?.configured ? t('status.configured') : t('status.notConfigured')}</span>
          </span>
        </div>

        <div className="upstream-selected-model cloud">
          <span className="upstream-selected-model-label">{t('upstream.currentModel')}</span>
          <span className="upstream-selected-model-value" id="upstream-cloud-selected-model">
            {getCloudModelDisplayName(cloudModel, autoLabel) || '—'}
          </span>
        </div>

        <div className="upstream-stats">
          <div className="stat-box cloud">
            <div className="label">{t('upstream.routeShare')}</div>
            <div className="value" id="upstream-cloud-route-pct">{cloudPct != null ? fmtPct(cloudPct) : '—'}</div>
            <div className="sub">{t('upstream.cloudRouteSub')}</div>
          </div>
          <div className="stat-box cloud">
            <div className="label">{t('upstream.tokenBudgetCap')}</div>
            <div className="value" id="upstream-cloud-budget">{quotaEnabled ? formatCompactNum(budgetValue, locale) : '—'}</div>
            <div className="sub" id="upstream-cloud-req">{cloudReq != null ? t('times', { n: cloudReq }) : '—'}</div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-title">{t('upstream.connection')}</div>
          <div className="upstream-form-grid">
            <div className="span-2">
              <label>{t('field.model')}</label>
              <ModelSelect
                id="cloud_model_picker"
                models={cloudModels}
                value={cloudModel}
                placeholder={t('ph.pickModel')}
                onChange={(v) => {
                  setCloudModel(v)
                  saveCloud(v, budgetSlider, quotaEnabled)
                }}
              />
              <input type="hidden" id="cloud_url" value={getCloudBaseUrl()} readOnly />
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-title">{t('upstream.limits')}</div>
          <div className="upstream-form-grid">
            <div className="upstream-form-col">
              <div className="switch-row">
                <span className="switch-row-label">{t('field.cloudQuotaEnabled')}</span>
                <label className="switch">
                  <input
                    type="checkbox"
                    id="cloud_quota_enabled"
                    checked={quotaEnabled}
                    onChange={(e) => {
                      const enabled = e.target.checked
                      setQuotaEnabled(enabled)
                      saveCloud(cloudModel, budgetSlider, enabled)
                    }}
                  />
                  <span className="switch-slider" />
                </label>
              </div>
              <div id="cloud-quota-fields" className={quotaEnabled ? '' : 'is-disabled'}>
                <label id="cloud_token_budget_label">{t('field.tokenBudget')}</label>
                <div className="budget-dragger">
                  <input type="hidden" id="cloud_token_budget" value={budgetValue} readOnly />
                  <input
                    id="cloud_token_budget_slider"
                    type="range"
                    min={0}
                    max={1000}
                    step={1}
                    value={budgetSlider}
                    aria-labelledby="cloud_token_budget_label"
                    onPointerDown={() => {
                      quotaEditingRef.current = true
                    }}
                    onChange={(e) => {
                      const v = Number(e.target.value)
                      setBudgetSlider(v)
                    }}
                    onPointerUp={(e) => {
                      saveCloud(cloudModel, Number(e.currentTarget.value), quotaEnabled)
                    }}
                    onKeyUp={(e) => {
                      if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'Home' || e.key === 'End') {
                        saveCloud(cloudModel, Number(e.currentTarget.value), quotaEnabled)
                      }
                    }}
                  />
                  <span className="budget-dragger-value" id="cloud_token_budget_value" aria-live="polite">
                    {formatCompactNum(budgetValue, locale)}
                  </span>
                </div>
                <div className="budget-dragger-limits">
                  <span id="cloud_token_budget_min">{formatCompactNum(CLOUD_BUDGET_MIN, locale)}</span>
                  <span id="cloud_token_budget_max">{formatCompactNum(CLOUD_BUDGET_MAX, locale)}</span>
                </div>
                <p className="hint">{t('upstream.budgetHint')}</p>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div id="edge-model-dialog" className={`security-dialog edge-model-dialog${dialogOpen ? ' open' : ''}`}>
        <div className="security-panel">
          <h3 id="edge-model-dialog-title">{editingEntry ? t('edgeModel.editTitle') : t('edgeModel.addTitle')}</h3>
          <div className="form-grid">
            <div>
              <label>{t('field.displayName')}</label>
              <input id="edge_dialog_name" placeholder={t('ph.edgeDisplayName')} value={dialogForm.name} onChange={(e) => setDialogForm((f) => ({ ...f, name: e.target.value }))} />
            </div>
            <div>
              <label>{t('field.baseUrl')}</label>
              <input id="edge_dialog_url" placeholder={t('ph.edgeUrl')} value={dialogForm.url} onChange={(e) => setDialogForm((f) => ({ ...f, url: e.target.value }))} />
              <p className="hint">{t('upstream.edgeUrlHint')}</p>
            </div>
            <div>
              <label>{t('field.model')}</label>
              <input id="edge_dialog_model" placeholder={t('ph.edgeModel')} value={dialogForm.model} onChange={(e) => setDialogForm((f) => ({ ...f, model: e.target.value }))} />
            </div>
            <div>
              <label>{t('field.maxContextWindow')}</label>
              <input
                id="edge_dialog_context_window"
                type="number"
                min={4096}
                max={2000000}
                step={1024}
                placeholder={t('ph.edgeContextWindow')}
                value={dialogForm.context_window}
                onChange={(e) => setDialogForm((f) => ({ ...f, context_window: e.target.value }))}
              />
              <p className="hint">{t('upstream.edgeContextHint')}</p>
            </div>
            <div>
              <label>{t('field.apiKey')}</label>
              <input id="edge_dialog_key" type="password" placeholder={t('ph.keepKey')} autoComplete="off" value={dialogForm.key} onChange={(e) => setDialogForm((f) => ({ ...f, key: e.target.value }))} />
              <p className="hint">{t('upstream.keyHint')}</p>
            </div>
          </div>
          <div className="security-actions">
            <button type="button" className="btn btn-ghost" id="edge-model-dialog-cancel" onClick={() => setDialogOpen(false)}>{t('action.cancel')}</button>
            <button type="button" className="btn btn-primary" id="edge-model-dialog-save" onClick={saveDialog}>{t('action.confirm')}</button>
          </div>
        </div>
      </div>
    </section>
  )
}
