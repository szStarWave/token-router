import { useEffect, useMemo, useRef, useState } from 'react'
import { useParams } from '@tanstack/react-router'
import { useSaveSetupMutation } from '../queries/gateway'
import { useCloudModelsQuery } from '../queries/flowy'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { useEdgeStore } from '../stores/edgeStore'
import { useCloudStore } from '../stores/cloudStore'
import { useI18n } from '../hooks/useI18n'
import { EdgeModelListItem } from '../components/upstream/EdgeModelListItem'
import { HerdsmanSetupBanner } from '../components/upstream/HerdsmanSetupBanner'
import { HerdsmanNoModelsBanner } from '../components/upstream/HerdsmanNoModelsBanner'
import {
  budgetFromSlider,
  buildCloudDisplayItems,
  fetchCloudModels,
  getCloudModelDisplayName,
  isCloudModelUiConfigured,
  persistCloudSelection,
  reconcileCloudModelSelection,
  resolveCloudModelSourceType,
  selectCloudModel,
  sliderFromCloudBudget,
  withAutoModelOption,
} from '../lib/cloud-upstream'
import {
  buildDisplayItems,
  deleteManualEntry,
  getEdgeModelDisplayName,
  isEdgeUpstreamConfigured,
  resolveEdgeModelSourceType,
  persistEdgeSelection,
  selectEdgeModel,
  upsertManualEntry,
  type ManualEdgeEntry,
} from '../lib/edge-upstream'
import { useOnboardingDemo } from '../lib/onboarding-demo'
import { fmtNum, fmtPct } from '../lib/stats-utils'
import type { UpstreamSetupView } from '../types/gateway'
import { DEFAULT_CLOUD_TOKEN_BUDGET, CLOUD_BUDGET_MIN, CLOUD_BUDGET_MAX } from '../constants/defaults'
import { formatCompactNum } from '../lib/format-number'
import { getAuthToken } from '../stores/authStore'

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
  const cloudSelectedKey = useCloudStore((s) => s.selectedKey)
  const cloudFlowyModels = useCloudStore((s) => s.flowyModels)
  const saveSetup = useSaveSetupMutation()
  const modelsQuery = useCloudModelsQuery()
  const demo = useOnboardingDemo()
  const demoActive = demo.active

  const effectiveSelectedKey = demoActive ? demo.edgeSelectedKey ?? selectedKey : selectedKey
  const effectiveCloudSelectedKey = demoActive ? demo.cloudSelectedKey ?? cloudSelectedKey : cloudSelectedKey

  const edgeConfigured = useMemo(
    () => demoActive || isEdgeUpstreamConfigured(setup?.edge),
    [demoActive, setup?.edge, herdsmanConnected, selectedKey, cachedModels],
  )

  const cloudConfigured = useMemo(
    () => demoActive || isCloudModelUiConfigured(setup?.cloud) || !!setup?.cloud?.configured,
    [demoActive, setup?.cloud, cloudSelectedKey, cloudFlowyModels],
  )

  const initialCloudQuota = cloudQuotaFromSetup(setup?.cloud)
  const [quotaEnabled, setQuotaEnabled] = useState(initialCloudQuota.enabled)
  const [budgetSlider, setBudgetSlider] = useState(() =>
    sliderFromCloudBudget(initialCloudQuota.budget > 0 ? initialCloudQuota.budget : DEFAULT_CLOUD_TOKEN_BUDGET),
  )
  const [edgeDialogOpen, setEdgeDialogOpen] = useState(false)
  const [editingEdgeEntry, setEditingEdgeEntry] = useState<ManualEdgeEntry | null>(null)
  const [edgeDialogForm, setEdgeDialogForm] = useState({ name: '', url: '', model: '', key: '', context_window: '' })
  const quotaEditingRef = useRef(false)
  const cloudSaveGenRef = useRef(0)

  useEffect(() => {
    if (quotaEditingRef.current) return
    const { budget, enabled } = cloudQuotaFromSetup(setup?.cloud)
    setQuotaEnabled(enabled)
    if (budget > 0) {
      setBudgetSlider(sliderFromCloudBudget(budget))
    }
  }, [setup?.cloud])

  useEffect(() => {
    if (modelsQuery.data) {
      useCloudStore.getState().setFlowyModels(withAutoModelOption(modelsQuery.data))
      reconcileCloudModelSelection()
    }
  }, [modelsQuery.data])

  useEffect(() => {
    if (!isEdge && connected && getAuthToken()) {
      void fetchCloudModels().catch(() => {})
    }
  }, [isEdge, connected])

  const edgePct = stats?.routing?.edge_pct
  const cloudPct = stats?.routing?.cloud_pct
  const edgeReq = stats?.routing?.edge
  const cloudReq = stats?.routing?.cloud

  const displayItems = buildDisplayItems()
  const herdsmanItems = displayItems.filter((i) => i.type === 'herdsman')
  const customEdgeItems = displayItems.filter((i) => i.type === 'manual')

  const cloudDisplayItems = buildCloudDisplayItems()
  const flowyItems = cloudDisplayItems.filter((i) => i.type === 'flowy')

  const demoHerdsmanItems = demoActive && !herdsmanItems.length
    ? demo.edgeModels.map((m) => ({
        key: `herdsman:${m.id}`,
        type: 'herdsman' as const,
        id: m.id,
        name: m.name,
        base_url: m.endpoint,
        model: m.id,
        context_window: m.context_window,
      }))
    : herdsmanItems

  const demoFlowyItems = demoActive && !flowyItems.length
    ? demo.cloudModels.map((m) => ({
        key: `flowy:${m.id}`,
        type: 'flowy' as const,
        id: m.id,
        name: m.name,
        base_url: 'https://api.flowy.ai/v1',
        model: m.id,
        icon: m.icon,
        context_window: m.context_window,
      }))
    : flowyItems

  const applyCloudQuotaFromSetup = (cloud: UpstreamSetupView['cloud']) => {
    const { budget, enabled } = cloudQuotaFromSetup(cloud)
    setQuotaEnabled(enabled)
    if (budget > 0) {
      setBudgetSlider(sliderFromCloudBudget(budget))
    }
  }

  const saveCloud = (slider: number, quota: boolean) => {
    if (demoActive || !connected) return
    quotaEditingRef.current = true
    const saveGen = ++cloudSaveGenRef.current
    const budget = quota ? budgetFromSlider(slider) : 0
    void persistCloudSelection(budget, (body) => {
      saveSetup.mutate(body, {
        onSuccess: (res) => {
          if (saveGen !== cloudSaveGenRef.current) return
          applyCloudQuotaFromSetup(res.upstream?.cloud)
          quotaEditingRef.current = false
        },
        onError: () => {
          if (saveGen === cloudSaveGenRef.current) quotaEditingRef.current = false
        },
      })
    })
  }

  const saveEdge = () => {
    if (demoActive) return
    void persistEdgeSelection((body) => saveSetup.mutate(body))
  }

  const openEdgeDialog = (entry?: ManualEdgeEntry) => {
    setEditingEdgeEntry(entry ?? null)
    setEdgeDialogForm({
      name: entry?.name ?? '',
      url: entry?.base_url ?? '',
      model: entry?.model ?? '',
      key: '',
      context_window: entry?.context_window != null ? String(entry.context_window) : '',
    })
    setEdgeDialogOpen(true)
  }

  const saveEdgeDialog = () => {
    const id = editingEdgeEntry?.id ?? `manual-${Date.now()}`
    const contextRaw = edgeDialogForm.context_window.trim()
    upsertManualEntry({
      id,
      name: edgeDialogForm.name.trim() || edgeDialogForm.model.trim(),
      base_url: edgeDialogForm.url.trim(),
      model: edgeDialogForm.model.trim(),
      api_key: edgeDialogForm.key.trim() || editingEdgeEntry?.api_key,
      context_window: contextRaw ? Number(contextRaw) : undefined,
    })
    setEdgeDialogOpen(false)
    saveEdge()
  }

  const budgetValue = budgetFromSlider(budgetSlider)
  const demoEdgeModelName = demoActive && demo.edgeModels.length > 0
    ? demo.edgeModels.find((m) => `herdsman:${m.id}` === demo.edgeSelectedKey)?.name ?? demo.edgeModels[0].name
    : null
  const edgeModelLabel = useMemo(
    () => demoEdgeModelName ?? getEdgeModelDisplayName(setup?.edge),
    [demoEdgeModelName, setup?.edge, herdsmanConnected, selectedKey, cachedModels],
  )
  const edgeModelTypeLabel = useMemo(() => {
    if (demoActive) return t('edgeModel.herdsman')
    const sourceType = resolveEdgeModelSourceType(setup?.edge)
    if (!sourceType) return ''
    return sourceType === 'herdsman' ? t('edgeModel.herdsman') : t('edgeModel.custom')
  }, [demoActive, setup?.edge, herdsmanConnected, selectedKey, cachedModels, t])
  const demoCloudModelName = demoActive && demo.cloudModels.length > 0
    ? demo.cloudModels.find((m) => `flowy:${m.id}` === demo.cloudSelectedKey)?.name ?? demo.cloudModels[0].name
    : null
  const cloudModelLabel = useMemo(
    () => demoCloudModelName ?? getCloudModelDisplayName(setup?.cloud?.model),
    [demoCloudModelName, setup?.cloud?.model, cloudSelectedKey, cloudFlowyModels],
  )
  const cloudModelTypeLabel = useMemo(() => {
    if (demoActive) return t('cloudModel.flowy')
    const sourceType = resolveCloudModelSourceType(setup?.cloud)
    if (!sourceType) return ''
    return sourceType === 'flowy' ? t('cloudModel.flowy') : t('cloudModel.custom')
  }, [demoActive, setup?.cloud, cloudSelectedKey, cloudFlowyModels, t])

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
          {edgeModelTypeLabel && (
            <span className="tag bordered upstream-selected-model-tag" id="upstream-edge-selected-model-tag">{edgeModelTypeLabel}</span>
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
              {!herdsmanConnected && !demoActive ? (
                <HerdsmanSetupBanner installed={herdsmanInstalled} />
              ) : !demoHerdsmanItems.length ? (
                <HerdsmanNoModelsBanner />
              ) : (
                demoHerdsmanItems.map((item) => (
                  <EdgeModelListItem
                    key={item.key}
                    item={item}
                    selected={effectiveSelectedKey === item.key}
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
              <button type="button" className="btn btn-primary btn-sm" id="btn-edge-model-add" onClick={() => openEdgeDialog()}>{t('action.add')}</button>
            </div>
            <div id="edge-custom-model-list" className="edge-model-list">
              {!customEdgeItems.length ? (
                <div className="edge-model-list-empty">{t('edgeModel.customEmpty')}</div>
              ) : (
                customEdgeItems.map((item) => (
                  <EdgeModelListItem
                    key={item.key}
                    item={item}
                    selected={selectedKey === item.key}
                    typeLabel={t('edgeModel.custom')}
                    selectLabel={item.name}
                    editLabel={t('action.edit')}
                    deleteLabel={t('action.delete')}
                    onSelect={() => { selectEdgeModel(item.key); saveEdge() }}
                    onEdit={() => openEdgeDialog(manualEntries.find((e) => e.id === item.id))}
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
          <span className={`tag bordered ${cloudConfigured ? 'ok' : 'off'} upstream-hero-status`} id="upstream-cloud-status">
            <span className="dot" />
            <span>{cloudConfigured ? t('status.configured') : t('status.notConfigured')}</span>
          </span>
        </div>

        <div className="upstream-selected-model cloud">
          <span className="upstream-selected-model-label">{t('upstream.currentModel')}</span>
          {cloudModelTypeLabel && (
            <span className="tag bordered upstream-selected-model-tag" id="upstream-cloud-selected-model-tag">{cloudModelTypeLabel}</span>
          )}
          <span className="upstream-selected-model-value" id="upstream-cloud-selected-model">
            {cloudModelLabel || '—'}
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
          <div className="panel-title">{t('upstream.cloudModelList')}</div>
          <div className="edge-model-subsection">
            <div className="edge-model-subsection-title">{t('cloudModel.flowyListTitle')}</div>
            <div id="cloud-flowy-model-list" className="edge-model-list">
              {!getAuthToken() && !demoActive ? (
                <div className="edge-model-list-empty">{t('cloudModel.flowyLoginRequired')}</div>
              ) : modelsQuery.isLoading && !demoFlowyItems.length ? (
                <div className="edge-model-list-empty">{t('status.loading')}</div>
              ) : !demoFlowyItems.length ? (
                <div className="edge-model-list-empty">{t('cloudModel.flowyEmpty')}</div>
              ) : (
                demoFlowyItems.map((item) => (
                  <EdgeModelListItem
                    key={item.key}
                    item={item}
                    selected={effectiveCloudSelectedKey === item.key}
                    typeLabel={t('cloudModel.flowy')}
                    selectLabel={item.name}
                    editLabel={t('action.edit')}
                    deleteLabel={t('action.delete')}
                    onSelect={() => { selectCloudModel(item.key); saveCloud(budgetSlider, quotaEnabled) }}
                  />
                ))
              )}
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
                      saveCloud(budgetSlider, enabled)
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
                      saveCloud(Number(e.currentTarget.value), quotaEnabled)
                    }}
                    onKeyUp={(e) => {
                      if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'Home' || e.key === 'End') {
                        saveCloud(Number(e.currentTarget.value), quotaEnabled)
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

      <div id="edge-model-dialog" className={`security-dialog edge-model-dialog${edgeDialogOpen ? ' open' : ''}`}>
        <div className="security-panel">
          <h3 id="edge-model-dialog-title">{editingEdgeEntry ? t('edgeModel.editTitle') : t('edgeModel.addTitle')}</h3>
          <div className="form-grid">
            <div>
              <label>{t('field.displayName')}</label>
              <input id="edge_dialog_name" placeholder={t('ph.edgeDisplayName')} value={edgeDialogForm.name} onChange={(e) => setEdgeDialogForm((f) => ({ ...f, name: e.target.value }))} />
            </div>
            <div>
              <label>{t('field.baseUrl')}</label>
              <input id="edge_dialog_url" placeholder={t('ph.edgeUrl')} value={edgeDialogForm.url} onChange={(e) => setEdgeDialogForm((f) => ({ ...f, url: e.target.value }))} />
              <p className="hint">{t('upstream.edgeUrlHint')}</p>
            </div>
            <div>
              <label>{t('field.model')}</label>
              <input id="edge_dialog_model" placeholder={t('ph.edgeModel')} value={edgeDialogForm.model} onChange={(e) => setEdgeDialogForm((f) => ({ ...f, model: e.target.value }))} />
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
                value={edgeDialogForm.context_window}
                onChange={(e) => setEdgeDialogForm((f) => ({ ...f, context_window: e.target.value }))}
              />
              <p className="hint">{t('upstream.edgeContextHint')}</p>
            </div>
            <div>
              <label>{t('field.apiKey')}</label>
              <input id="edge_dialog_key" type="password" placeholder={t('ph.keepKey')} autoComplete="off" value={edgeDialogForm.key} onChange={(e) => setEdgeDialogForm((f) => ({ ...f, key: e.target.value }))} />
              <p className="hint">{t('upstream.keyHint')}</p>
            </div>
          </div>
          <div className="security-actions">
            <button type="button" className="btn btn-ghost" id="edge-model-dialog-cancel" onClick={() => setEdgeDialogOpen(false)}>{t('action.cancel')}</button>
            <button type="button" className="btn btn-primary" id="edge-model-dialog-save" onClick={saveEdgeDialog}>{t('action.confirm')}</button>
          </div>
        </div>
      </div>
    </section>
  )
}
