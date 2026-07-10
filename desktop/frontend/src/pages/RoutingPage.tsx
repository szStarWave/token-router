import { useEffect, useState } from 'react'
import { useSaveSetupMutation } from '../queries/gateway'
import { useSetupStore } from '../stores/setupStore'
import { useAppStore } from '../stores/appStore'
import { useI18n } from '../hooks/useI18n'
import type { GatewayConfigView } from '../types/gateway'

const DEFAULTS: Partial<GatewayConfigView> = {
  route: 'auto',
  routing_mode: 'cascade',
  default_profile: 'economy',
  ctx_edge_max_tokens: 200000,
  experience_enabled: false,
  adaptive_routing_enabled: false,
  classifier_enabled: false,
  classifier_prior_from_heuristic: true,
}

export function RoutingPage() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const setup = useSetupStore((s) => s.setup)
  const saveSetup = useSaveSetupMutation()
  const g = setup?.gateway

  const [form, setForm] = useState<Partial<GatewayConfigView>>({})

  useEffect(() => {
    if (g) setForm({ ...g })
  }, [g])

  const set = <K extends keyof GatewayConfigView>(key: K, val: GatewayConfigView[K]) => {
    setForm((f) => ({ ...f, [key]: val }))
  }

  const saveRouting = () => {
    if (!connected) {
      showToast('conn.offline', false)
      return
    }
    saveSetup.mutate({ gateway: form }, {
      onSuccess: () => showToast('toast.routingSaved'),
    })
  }

  const resetDefaults = () => {
    setForm({ ...DEFAULTS })
    if (connected) {
      saveSetup.mutate({ gateway: DEFAULTS }, {
        onSuccess: () => showToast('toast.resetOk'),
      })
    }
  }

  const routes = ['auto', 'edge', 'cloud', 'cascade'] as const
  const routingModes = ['single', 'cascade', 'split'] as const
  const profiles = ['economy', 'balanced', 'premium', 'privacy'] as const

  return (
    <section className="page active" id="page-routing">
      <div className="page-routing-scroll">
        <div className="panel">
          <div className="panel-title">{t('routing.mode')}</div>
          <div className="form-row">
            <div>
              <label>{t('field.route')}</label>
              <select id="route" value={form.route ?? 'auto'} onChange={(e) => set('route', e.target.value as GatewayConfigView['route'])}>
                {routes.map((x) => (
                  <option key={x} value={x}>{t(`opt.route.${x}`)}</option>
                ))}
              </select>
            </div>
            <div>
              <label>{t('field.routingMode')}{t('field.routingModeHint')}</label>
              <select id="routing_mode" value={form.routing_mode ?? 'cascade'} onChange={(e) => set('routing_mode', e.target.value as GatewayConfigView['routing_mode'])}>
                {routingModes.map((x) => (
                  <option key={x} value={x}>{t(`opt.routingMode.${x}`)}</option>
                ))}
              </select>
            </div>
            <div>
              <label>{t('field.defaultProfile')}</label>
              <select id="default_profile" value={form.default_profile ?? 'balanced'} onChange={(e) => set('default_profile', e.target.value as GatewayConfigView['default_profile'])}>
                {profiles.map((x) => (
                  <option key={x} value={x}>{t(`opt.profile.${x}`)}</option>
                ))}
              </select>
            </div>
            <div>
              <label>{t('field.ctxEdgeMaxTokens')}</label>
              <input id="ctx_edge_max" type="number" min={4096} max={2000000} step={1024} placeholder="200000" value={form.ctx_edge_max_tokens ?? ''} onChange={(e) => set('ctx_edge_max_tokens', e.target.value ? Number(e.target.value) : undefined)} />
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-title">{t('routing.experience')}</div>
          <div className="form-switches">
            <div className="switch-row">
              <label htmlFor="experience_enabled">{t('field.experienceEnabled')}</label>
              <label className="switch" aria-hidden="true">
                <input type="checkbox" id="experience_enabled" checked={!!form.experience_enabled} onChange={(e) => set('experience_enabled', e.target.checked)} />
                <span className="switch-slider" />
              </label>
            </div>
          </div>
          <div className="form-row">
            <div><label>{t('field.workVerifySampleRate')}</label><input id="work_verify_sample_rate" type="number" min={0} max={1} step={0.05} value={form.work_verify_sample_rate ?? ''} onChange={(e) => set('work_verify_sample_rate', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.cloudCacheDecayHalfLifeSecs')}</label><input id="cloud_cache_decay_half_life_secs" type="number" min={0} step={60} value={form.cloud_cache_decay_half_life_secs ?? form.cloud_sticky_ttl_secs ?? ''} onChange={(e) => set('cloud_cache_decay_half_life_secs', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.cloudCacheBoostMax')}</label><input id="cloud_cache_boost_max" type="number" min={0} max={1} step={0.01} value={form.cloud_cache_boost_max ?? ''} onChange={(e) => set('cloud_cache_boost_max', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.experienceLearningRate')}</label><input id="experience_learning_rate" type="number" min={0} max={1} step={0.01} value={form.experience_learning_rate ?? ''} onChange={(e) => set('experience_learning_rate', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.experienceTargetFallback')}</label><input id="experience_target_fallback" type="number" min={0} max={1} step={0.01} value={form.experience_target_fallback ?? ''} onChange={(e) => set('experience_target_fallback', e.target.value ? Number(e.target.value) : undefined)} /></div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-title">{t('routing.adaptive')}</div>
          <p className="hint">{t('routing.adaptiveHint')}</p>
          <div className="form-switches">
            <div className="switch-row">
              <label htmlFor="adaptive_routing_enabled">{t('field.adaptiveRoutingEnabled')}</label>
              <label className="switch" aria-hidden="true">
                <input type="checkbox" id="adaptive_routing_enabled" checked={!!form.adaptive_routing_enabled} onChange={(e) => set('adaptive_routing_enabled', e.target.checked)} />
                <span className="switch-slider" />
              </label>
            </div>
          </div>
          <div className="form-row">
            <div><label>{t('field.adaptiveMinVerifiedSamples')}</label><input id="adaptive_min_verified_samples" type="number" min={1} max={1000000} step={1} placeholder="20" value={form.adaptive_min_verified_samples ?? ''} onChange={(e) => set('adaptive_min_verified_samples', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.adaptiveVerifyRateFloor')}</label><input id="adaptive_verify_rate_floor" type="number" min={0} max={1} step={0.01} placeholder="0.05" value={form.adaptive_verify_rate_floor ?? ''} onChange={(e) => set('adaptive_verify_rate_floor', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.adaptiveVerifyRateCeiling')}</label><input id="adaptive_verify_rate_ceiling" type="number" min={0} max={1} step={0.01} placeholder="0.45" value={form.adaptive_verify_rate_ceiling ?? ''} onChange={(e) => set('adaptive_verify_rate_ceiling', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.adaptiveMaxThetaShift')}</label><input id="adaptive_max_theta_shift" type="number" min={0} max={0.5} step={0.01} placeholder="0.05" value={form.adaptive_max_theta_shift ?? ''} onChange={(e) => set('adaptive_max_theta_shift', e.target.value ? Number(e.target.value) : undefined)} /></div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-title">{t('routing.classifier')}</div>
          <p className="hint">{t('routing.classifierHint')}</p>
          <div className="form-switches">
            <div className="switch-row">
              <label htmlFor="classifier_enabled">{t('field.classifierEnabled')}</label>
              <label className="switch" aria-hidden="true">
                <input type="checkbox" id="classifier_enabled" checked={!!form.classifier_enabled} onChange={(e) => set('classifier_enabled', e.target.checked)} />
                <span className="switch-slider" />
              </label>
            </div>
            <div className="switch-row">
              <label htmlFor="classifier_prior_from_heuristic">{t('field.classifierPriorFromHeuristic')}</label>
              <label className="switch" aria-hidden="true">
                <input type="checkbox" id="classifier_prior_from_heuristic" checked={form.classifier_prior_from_heuristic !== false} onChange={(e) => set('classifier_prior_from_heuristic', e.target.checked)} />
                <span className="switch-slider" />
              </label>
            </div>
          </div>
          <div className="form-row">
            <div><label>{t('field.classifierMinSamples')}</label><input id="classifier_min_samples" type="number" min={1} max={1000000} step={1} placeholder="100" value={form.classifier_min_samples ?? ''} onChange={(e) => set('classifier_min_samples', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.classifierPriorAlpha')}</label><input id="classifier_prior_alpha" type="number" min={0} max={100} step={0.1} placeholder="1.0" value={form.classifier_prior_alpha ?? ''} onChange={(e) => set('classifier_prior_alpha', e.target.value ? Number(e.target.value) : undefined)} /></div>
            <div><label>{t('field.classifierDecayHalfLife')}</label><input id="classifier_decay_half_life_hours" type="number" min={0} step={1} placeholder="168" value={form.classifier_decay_half_life_hours ?? ''} onChange={(e) => set('classifier_decay_half_life_hours', e.target.value ? Number(e.target.value) : undefined)} /></div>
          </div>
        </div>
      </div>

      <div className="page-routing-footer upstream-actions">
        <button type="button" className="btn btn-primary" onClick={saveRouting}>{t('action.saveRouting')}</button>
        <button type="button" className="btn btn-ghost" onClick={resetDefaults}>{t('action.resetDefault')}</button>
      </div>
    </section>
  )
}
