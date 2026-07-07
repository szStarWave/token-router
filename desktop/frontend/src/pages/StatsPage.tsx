import { useState } from 'react'
import { useAppStore } from '../stores/appStore'
import { useI18n } from '../hooks/useI18n'
import { StatsChart, StatsChartRange, type ChartRange } from '../components/stats/StatsChart'
import { AuthKeyStatsPanel } from '../components/stats/AuthKeyStatsPanel'
import { classifierFeatureLabel, classifierSummaryRows, fmtMs, fmtNum, fmtPct, fmtTps, formatClassifierSummaryValue, formatSavedCreditsAmount, stepKindLabel, tierMaxPerRequest, tierTokenSummary, tokenSummary, tokenTableRows, topStepKinds } from '../lib/stats-utils'
import type { StatsScope } from '../types/gateway'

export function StatsPage() {
  const { t, locale } = useI18n()
  const scope = useAppStore((s) => s.scope)
  const setScope = useAppStore((s) => s.setScope)
  const stats = useAppStore((s) => s.stats)
  const sessionSavedPoints = useAppStore((s) => s.sessionSavedPoints)
  const globalSavedPoints = useAppStore((s) => s.globalSavedPoints)
  const savedPoints = scope === 'session' ? sessionSavedPoints : globalSavedPoints
  const [chartRange, setChartRange] = useState<ChartRange>('h24')

  const routing = stats?.routing
  const cascade = stats?.cascade
  const latency = stats?.latency
  const tb = stats?.token_breakdown as Record<string, unknown> | undefined
  const adaptive = stats?.effective_routing as Record<string, unknown> | null | undefined
  const classifier = stats?.classifier as Record<string, unknown> | null | undefined

  const stepKinds = topStepKinds(stats)
  const tokens = tokenTableRows(tb)
  const tokenTotals = tokenSummary(tb)
  const edgeTokens = tierTokenSummary(tb?.edge as Record<string, unknown> | undefined)
  const cloudTokens = tierTokenSummary(tb?.cloud as Record<string, unknown> | undefined)
  const edgeMaxTokens = tierMaxPerRequest(tb?.edge as Record<string, unknown> | undefined)
  const cloudMaxTokens = tierMaxPerRequest(tb?.cloud as Record<string, unknown> | undefined)
  const p95 = latency?.p95_ms
  const p99 = latency?.p99_ms
  const classifierFeatures = (classifier?.top_cloud_features as Array<{
    feature: string
    edge_ok: number
    cloud_needed: number
    cloud_rate: number
  }> | undefined) ?? []
  const classifierRows = classifier ? classifierSummaryRows(classifier) : []

  const onScope = (s: StatsScope) => {
    setScope(s)
  }

  return (
    <section className="page active" id="page-stats">
      <div className="segment-tabs stats-scope-tabs" role="tablist" aria-label={t('stats.scopeTabs')}>
        <button
          type="button"
          role="tab"
          aria-selected={scope === 'global'}
          className={`segment-tab${scope === 'global' ? ' active' : ''}`}
          data-scope="global"
          onClick={() => onScope('global')}
        >
          {t('stats.global')}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={scope === 'session'}
          className={`segment-tab${scope === 'session' ? ' active' : ''}`}
          data-scope="session"
          onClick={() => onScope('session')}
        >
          {t('stats.session')}
        </button>
      </div>

      <div className="stat-grid">
        <div className="stat-box">
          <div className="label">{t('stat.savedPrefix')}</div>
          <div className="value" id="stat-saved">{formatSavedCreditsAmount(savedPoints, locale)}</div>
          <div className="sub">{t('stat.creditsUnit')}</div>
        </div>
        <div className="stat-box">
          <div className="label">{t('stat.totalReq')}</div>
          <div className="value" id="stat-req">{fmtNum(stats?.requests_total, locale)}</div>
          <div className="sub" id="stat-rpm">{stats?.requests_per_minute != null ? t('stat.reqPerMin', { n: Math.round(stats.requests_per_minute) }) : '—'}</div>
        </div>
        <div className="stat-box">
          <div className="label">{t('stat.tokens')}</div>
          <div className="value" id="stat-tokens">{tokenTotals.total > 0 ? fmtNum(tokenTotals.total, locale) : '—'}</div>
          <div className="sub" id="stat-token-io">{t('stat.tokenInOut', { input: fmtNum(tokenTotals.input, locale), output: fmtNum(tokenTotals.output, locale) })}</div>
        </div>
        <div className="stat-box">
          <div className="label">{t('stat.p95')}</div>
          <div className="value" id="stat-p95">{p95 != null && p95 > 0 ? fmtMs(p95, t) : '—'}</div>
          <div className="sub" id="stat-p99">{p99 != null && p99 > 0 ? t('stat.p99Sub', { n: Math.round(p99) }) : '—'}</div>
        </div>
        <div className="stat-box">
          <div className="label">{t('stat.avgTtft')}</div>
          <div className="value" id="stat-ttft">{fmtMs(latency?.avg_ttft_ms, t)}</div>
          <div className="sub">{t('stat.ttftSub')}</div>
        </div>
        <div className="stat-box">
          <div className="label">{t('stat.avgTps')}</div>
          <div className="value" id="stat-avg-tps">{fmtTps(latency?.avg_tps)}</div>
          <div className="sub">{t('stat.tpsUnit')}</div>
        </div>
        <div className="stat-box cloud">
          <div className="label">{t('stat.cloudRoute')}</div>
          <div className="value" id="stat-cloud-pct">{routing ? fmtPct(routing.cloud_pct) : '—'}</div>
          <div className="sub" id="stat-cloud-n">{routing ? t('times', { n: routing.cloud }) : '—'}</div>
        </div>
        <div className="stat-box cloud">
          <div className="label">{t('stat.cloudTokens')}</div>
          <div className="value" id="stat-cloud-tokens">{cloudTokens.total > 0 ? fmtNum(cloudTokens.total, locale) : '—'}</div>
          <div className="sub" id="stat-cloud-token-io">{t('stat.tokenInOut', { input: fmtNum(cloudTokens.input, locale), output: fmtNum(cloudTokens.output, locale) })}</div>
        </div>
        <div className="stat-box cloud">
          <div className="label">{t('stat.cloudTps')}</div>
          <div className="value" id="stat-cloud-tps">{fmtTps(latency?.cloud_tps)}</div>
          <div className="sub">{t('stat.tpsUnit')}</div>
        </div>
        <div className="stat-box cloud">
          <div className="label">{t('stat.cloudMaxOutputPerReq')}</div>
          <div className="value" id="stat-cloud-max-out">{cloudMaxTokens.output > 0 ? fmtNum(cloudMaxTokens.output, locale) : '—'}</div>
          <div className="sub" id="stat-cloud-max-out-foot">{cloudMaxTokens.output > 0 ? t('stat.maxOutputReqFoot', { total: fmtNum(cloudMaxTokens.atMaxOutput.total, locale), input: fmtNum(cloudMaxTokens.atMaxOutput.input, locale) }) : '—'}</div>
        </div>
        <div className="stat-box cloud">
          <div className="label">{t('stat.cloudMaxInputPerReq')}</div>
          <div className="value" id="stat-cloud-max-in">{cloudMaxTokens.input > 0 ? fmtNum(cloudMaxTokens.input, locale) : '—'}</div>
          <div className="sub" id="stat-cloud-max-in-foot">{cloudMaxTokens.input > 0 ? t('stat.maxInputReqFoot', { total: fmtNum(cloudMaxTokens.atMaxInput.total, locale), output: fmtNum(cloudMaxTokens.atMaxInput.output, locale) }) : '—'}</div>
        </div>
        <div className="stat-box edge">
          <div className="label">{t('stat.edgeRoute')}</div>
          <div className="value" id="stat-edge-pct">{routing ? fmtPct(routing.edge_pct) : '—'}</div>
          <div className="sub" id="stat-edge-n">{routing ? t('times', { n: routing.edge }) : '—'}</div>
        </div>
        <div className="stat-box edge">
          <div className="label">{t('stat.edgeTokens')}</div>
          <div className="value" id="stat-edge-tokens">{edgeTokens.total > 0 ? fmtNum(edgeTokens.total, locale) : '—'}</div>
          <div className="sub" id="stat-edge-token-io">{t('stat.tokenInOut', { input: fmtNum(edgeTokens.input, locale), output: fmtNum(edgeTokens.output, locale) })}</div>
        </div>
        <div className="stat-box edge">
          <div className="label">{t('stat.edgeTps')}</div>
          <div className="value" id="stat-edge-tps">{fmtTps(latency?.edge_tps)}</div>
          <div className="sub">{t('stat.tpsUnit')}</div>
        </div>
        <div className="stat-box edge">
          <div className="label">{t('stat.edgeMaxOutputPerReq')}</div>
          <div className="value" id="stat-edge-max-out">{edgeMaxTokens.output > 0 ? fmtNum(edgeMaxTokens.output, locale) : '—'}</div>
          <div className="sub" id="stat-edge-max-out-foot">{edgeMaxTokens.output > 0 ? t('stat.maxOutputReqFoot', { total: fmtNum(edgeMaxTokens.atMaxOutput.total, locale), input: fmtNum(edgeMaxTokens.atMaxOutput.input, locale) }) : '—'}</div>
        </div>
        <div className="stat-box edge">
          <div className="label">{t('stat.edgeMaxInputPerReq')}</div>
          <div className="value" id="stat-edge-max-in">{edgeMaxTokens.input > 0 ? fmtNum(edgeMaxTokens.input, locale) : '—'}</div>
          <div className="sub" id="stat-edge-max-in-foot">{edgeMaxTokens.input > 0 ? t('stat.maxInputReqFoot', { total: fmtNum(edgeMaxTokens.atMaxInput.total, locale), output: fmtNum(edgeMaxTokens.atMaxInput.output, locale) }) : '—'}</div>
        </div>
      </div>

      <div className="panel" style={{ marginTop: 16 }}>
        <div className="panel-title">{t('overview.routeDist')}</div>
        <div className="route-chart">
          <div className="progress-edge" id="chart-edge" style={{ width: `${routing?.edge_pct ?? 0}%` }} />
          <div className="progress-cascade" id="chart-cascade" style={{ width: `${routing?.cascade_pct ?? 0}%` }} />
          <div className="progress-cloud" id="chart-cloud" style={{ width: `${routing?.cloud_pct ?? 0}%` }} />
        </div>
        <div className="route-legend">
          <span><span className="legend-dot progress-edge" /> {t('legend.edge')} <strong id="leg-edge">{routing ? fmtPct(routing.edge_pct) : '0%'}</strong></span>
          <span><span className="legend-dot progress-cascade" /> {t('legend.cascade')} <strong id="leg-cascade">{routing ? fmtPct(routing.cascade_pct) : '0%'}</strong></span>
          <span><span className="legend-dot progress-cloud" /> {t('legend.cloud')} <strong id="leg-cloud">{routing ? fmtPct(routing.cloud_pct) : '0%'}</strong></span>
        </div>
      </div>

      <div className="panel">
        <div className="panel-title">{t('overview.stepKinds')}</div>
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{t('col.stepKind')}</th>
                <th className="num">{t('col.count')}</th>
                <th className="num">{t('col.share')}</th>
              </tr>
            </thead>
            <tbody id="step-kind-table">
              {stepKinds.map((row) => (
                <tr key={row.kind}>
                  <td>{stepKindLabel(row.kind, t)}</td>
                  <td className="num">{row.count.toLocaleString()}</td>
                  <td className="num">{fmtPct(row.pct)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      <div className="panel">
        <div className="token-chart-header">
          <div className="panel-title" style={{ marginBottom: 0 }}>{t('stats.tokenTrend')}</div>
          <StatsChartRange range={chartRange} onChange={setChartRange} />
        </div>
        <StatsChart scope={scope} range={chartRange} />
      </div>

      <div className="panel">
        <div className="panel-title">{t('stats.token')}</div>
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{t('col.metric')}</th>
                <th className="num">{t('col.edge')}</th>
                <th className="num">{t('col.cloud')}</th>
                <th className="num">{t('col.total')}</th>
              </tr>
            </thead>
            <tbody id="token-table">
              {tokens.map((row) => (
                <tr key={row.key}>
                  <td>{t(`stat.${row.key}Token` as 'stat.inputToken')}</td>
                  <td className="num">{fmtNum(row.edge, locale)}</td>
                  <td className="num">{fmtNum(row.cloud, locale)}</td>
                  <td className="num">{fmtNum(row.total, locale)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {(stats?.auth_key_stats?.length ?? 0) > 0 && (
        <AuthKeyStatsPanel keys={stats!.auth_key_stats!} />
      )}

      <div className="panel">
        <div className="panel-title">{t('stats.cascadeLatency')}</div>
        <div className="stat-grid" id="stats-detail">
          <div className="stat-box"><div className="label">{t('stat.cascadeEdgeOk')}</div><div className="value">{cascade?.edge_ok ?? '—'}</div></div>
          <div className="stat-box"><div className="label">{t('stat.cascadeFallback')}</div><div className="value">{cascade?.fallback_to_cloud ?? '—'}</div></div>
          <div className="stat-box"><div className="label">{t('stat.avgReqMs')}</div><div className="value">{fmtMs(latency?.avg_request_ms, t)}</div></div>
        </div>
      </div>

      {adaptive && Object.keys(adaptive).length > 0 && (
        <div className="panel" id="adaptive-panel">
          <div className="panel-title">{t('stats.adaptive')}</div>
          <div className="table-wrap">
            <table>
              <thead><tr><th>{t('col.metric')}</th><th className="num">{t('col.value')}</th></tr></thead>
              <tbody id="adaptive-table">
                {Object.entries(adaptive).map(([k, v]) => (
                  <tr key={k}><td>{t(`adaptive.${k}` as 'adaptive.enabled')}</td><td className="num">{String(v)}</td></tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {classifier && (
        <div className="panel" id="classifier-panel">
          <div className="panel-title">{t('stats.classifier')}</div>
          <div className="stat-grid" id="classifier-summary">
            {classifierRows.map(({ key, value }) => (
              <div key={key} className="stat-box">
                <div className="label">{t(`classifier.${key}` as 'classifier.enabled')}</div>
                <div className="value">{formatClassifierSummaryValue(key, value, t)}</div>
              </div>
            ))}
          </div>
          <div className="table-wrap" id="classifier-features-wrap" style={{ marginTop: 12 }}>
            <div className="panel-title" style={{ marginBottom: 8, fontSize: 13 }}>{t('classifier.topCloudFeatures')}</div>
            <table>
              <thead>
                <tr>
                  <th>{t('col.feature')}</th>
                  <th className="num">{t('col.edgeOk')}</th>
                  <th className="num">{t('col.cloudNeeded')}</th>
                  <th className="num">{t('col.cloudRate')}</th>
                </tr>
              </thead>
              <tbody id="classifier-features-table">
                {classifierFeatures.length ? (
                  classifierFeatures.slice(0, 8).map((f) => (
                    <tr key={f.feature}>
                      <td>
                        <code title={f.feature}>{classifierFeatureLabel(f.feature, t)}</code>
                      </td>
                      <td className="num">{(f.edge_ok ?? 0).toLocaleString()}</td>
                      <td className="num">{(f.cloud_needed ?? 0).toLocaleString()}</td>
                      <td className="num">{fmtPct((f.cloud_rate ?? 0) * 100)}</td>
                    </tr>
                  ))
                ) : (
                  <tr>
                    <td colSpan={4} style={{ color: 'var(--default-400)' }}>{t('classifier.noFeatures')}</td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </section>
  )
}
