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
  deleteManualCloudEntry,
  fetchCloudModels,
  getCloudModelDisplayName,
  isCloudModelUiConfigured,
  persistCloudSelection,
  reconcileCloudModelSelection,
  selectCloudModel,
  sliderFromCloudBudget,
  upsertManualCloudEntry,
  withAutoModelOption,
  type ManualCloudEntry,
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
  const cloudManualEntries = useCloudStore((s) => s.manualEntries)
  const cloudFlowyModels = useCloudStore((s) => s.flowyModels)
  const saveSetup = useSaveSetupMutation()
  const modelsQuery = useCloudModelsQuery()

  const edgeConfigured = useMemo(
    () => isEdgeUpstreamConfigured(setup?.edge),
    [setup?.edge, herdsmanConnected, selectedKey, cachedModels],
  )

  const autoLabel = t('cloudModel.auto')
  const cloudConfigured = useMemo(
    () => isCloudModelUiConfigured(setup?.cloud) || !!setup?.cloud?.configured,
    [setup?.cloud, cloudSelectedKey, cloudManualEntries, cloudFlowyModels],
  )

  const initialCloudQuota = cloudQuotaFromSetup(setup?.cloud)
  const [quotaEnabled, setQuotaEnabled] = useState(initialCloudQuota.enabled)
  const [budgetSlider, setBudgetSlider] = useState(() =>
    sliderFromCloudBudget(initialCloudQuota.budget > 0 ? initialCloudQuota.budget : DEFAULT_CLOUD_TOKEN_BUDGET),
  )
  const [edgeDialogOpen, setEdgeDialogOpen] = useState(false)
  const [cloudDialogOpen, setCloudDialogOpen] = useState(false)
  const [editingEdgeEntry, setEditingEdgeEntry] = useState<ManualEdgeEntry | null>(null)
  const [editingCloudEntry, setEditingCloudEntry] = useState<ManualCloudEntry | null>(null)
  const [edgeDialogForm, setEdgeDialogForm] = useState({ name: '', url: '', model: '', key: '', context_window: '' })
  const [cloudDialogForm, setCloudDialogForm] = useState({ name: '', url: '', model: '', key: '' })
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
      useCloudStore.getState().setFlowyModels(withAutoModelOption(modelsQuery.data, autoLabel))
      reconcileCloudModelSelection()
    }
  }, [modelsQuery.data, autoLabel])

  useEffect(() => {
    if (!isEdge && connected && getAuthToken()) {
      void fetchCloudModels(autoLabel).catch(() => {})
    }
  }, [isEdge, connected, autoLabel])

  const edgePct = stats?.routing?.edge_pct
  const cloudPct = stats?.routing?.cloud_pct
  const edgeReq = stats?.routing?.edge
  const cloudReq = stats?.routing?.cloud

  const displayItems = buildDisplayItems()
  const herdsmanItems = displayItems.filter((i) => i.type === 'herdsman')
  const customEdgeItems = displayItems.filter((i) => i.type === 'manual')

  const cloudDisplayItems = buildCloudDisplayItems()
  const flowyItems = cloudDisplayItems.filter((i) => i.type === 'flowy')
  const customCloudItems = cloudDisplayItems.filter((i) => i.type === 'manual')

  const applyCloudQuotaFromSetup = (cloud: UpstreamSetupView['cloud']) => {
    const { budget, enabled } = cloudQuotaFromSetup(cloud)
    setQuotaEnabled(enabled)
    if (budget > 0) {
      setBudgetSlider(sliderFromCloudBudget(budget))
    }
  }

  const saveCloud = (slider: number, quota: boolean) => {
    if (!connected) return
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

  const openCloudDialog = (entry?: ManualCloudEntry) => {
    setEditingCloudEntry(entry ?? null)
    setCloudDialogForm({
      name: entry?.name ?? '',
      url: entry?.base_url ?? '',
      model: entry?.model ?? '',
      key: '',
    })
    setCloudDialogOpen(true)
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

  const saveCloudDialog = () => {
    const id = editingCloudEntry?.id ?? `cloud-manual-${Date.now()}`
    upsertManualCloudEntry({
      id,
      name: cloudDialogForm.name.trim() || cloudDialogForm.model.trim(),
      base_url: cloudDialogForm.url.trim(),
      model: cloudDialogForm.model.trim(),
      api_key: cloudDialogForm.key.trim() || editingCloudEntry?.api_key,
    })
    setCloudDialogOpen(false)
    saveCloud(budgetSlider, quotaEnabled)
  }

  const budgetValue = budgetFromSlider(budgetSlider)
  const edgeModelLabel = useMemo(
    () => getEdgeModelDisplayName(setup?.edge),
    [setup?.edge, herdsmanConnected, selectedKey, cachedModels],
  )
  const cloudModelLabel = useMemo(
    () => getCloudModelDisplayName(setup?.cloud?.model, autoLabel),
    [setup?.cloud?.model, autoLabel, cloudSelectedKey, cloudManualEntries, cloudFlowyModels],
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
                <HerdsmanNoModelsBanner />
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
              {!getAuthToken() ? (
                <div className="edge-model-list-empty">{t('cloudModel.flowyLoginRequired')}</div>
              ) : modelsQuery.isLoading && !flowyItems.length ? (
                <div className="edge-model-list-empty">{t('status.loading')}</div>
              ) : !flowyItems.length ? (
                <div className="edge-model-list-empty">{t('cloudModel.flowyEmpty')}</div>
              ) : (
                flowyItems.map((item) => (
                  <EdgeModelListItem
                    key={item.key}
                    item={item}
                    selected={cloudSelectedKey === item.key}
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
          <div className="edge-model-subsection">
            <div className="edge-model-list-header">
              <div className="edge-model-subsection-title">{t('cloudModel.customListTitle')}</div>
              <button type="button" className="btn btn-primary btn-sm" id="btn-cloud-model-add" onClick={() => openCloudDialog()}>{t('action.add')}</button>
            </div>
            <div id="cloud-custom-model-list" className="edge-model-list">
              {!customCloudItems.length ? (
                <div className="edge-model-list-empty">{t('cloudModel.customEmpty')}</div>
              ) : (
                customCloudItems.map((item) => (
                  <EdgeModelListItem
                    key={item.key}
                    item={item}
                    selected={cloudSelectedKey === item.key}
                    typeLabel={t('cloudModel.custom')}
                    selectLabel={item.name}
                    editLabel={t('action.edit')}
                    deleteLabel={t('action.delete')}
                    onSelect={() => { selectCloudModel(item.key); saveCloud(budgetSlider, quotaEnabled) }}
                    onEdit={() => openCloudDialog(cloudManualEntries.find((e) => e.id === item.id))}
                    onDelete={() => { deleteManualCloudEntry(item.id); saveCloud(budgetSlider, quotaEnabled) }}
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

      <div id="cloud-model-dialog" className={`security-dialog edge-model-dialog${cloudDialogOpen ? ' open' : ''}`}>
        <div className="security-panel">
          <h3 id="cloud-model-dialog-title">{editingCloudEntry ? t('cloudModel.editTitle') : t('cloudModel.addTitle')}</h3>
          <div className="form-grid">
            <div>
              <label>{t('field.displayName')}</label>
              <input id="cloud_dialog_name" placeholder={t('ph.edgeDisplayName')} value={cloudDialogForm.name} onChange={(e) => setCloudDialogForm((f) => ({ ...f, name: e.target.value }))} />
            </div>
            <div>
              <label>{t('field.baseUrl')}</label>
              <input id="cloud_dialog_url" placeholder={t('ph.cloudUrl')} value={cloudDialogForm.url} onChange={(e) => setCloudDialogForm((f) => ({ ...f, url: e.target.value }))} />
              <p className="hint">{t('upstream.cloudUrlHint')}</p>
            </div>
            <div>
              <label>{t('field.model')}</label>
              <input id="cloud_dialog_model" placeholder={t('ph.cloudModel')} value={cloudDialogForm.model} onChange={(e) => setCloudDialogForm((f) => ({ ...f, model: e.target.value }))} />
              <p className="hint">{t('upstream.cloudModelHint')}</p>
            </div>
            <div>
              <label>{t('field.apiKey')}</label>
              <input id="cloud_dialog_key" type="password" placeholder={t('ph.keepKey')} autoComplete="off" value={cloudDialogForm.key} onChange={(e) => setCloudDialogForm((f) => ({ ...f, key: e.target.value }))} />
              <p className="hint">{t('upstream.keyHint')}</p>
            </div>
          </div>
          <div className="security-actions">
            <button type="button" className="btn btn-ghost" id="cloud-model-dialog-cancel" onClick={() => setCloudDialogOpen(false)}>{t('action.cancel')}</button>
            <button type="button" className="btn btn-primary" id="cloud-model-dialog-save" onClick={saveCloudDialog}>{t('action.confirm')}</button>
          </div>
        </div>
      </div>
    </section>
  )
}
