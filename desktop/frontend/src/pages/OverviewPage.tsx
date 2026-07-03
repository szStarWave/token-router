import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { useI18n } from '../hooks/useI18n'
import { useLiveUptime } from '../hooks/useLiveUptime'
import { formatUptime } from '../lib/stats-utils'
import { AgentQuickSetupCard } from '../components/overview/AgentQuickSetupCard'
import { CcSwitchExportCard } from '../components/overview/CcSwitchExportCard'

interface EndpointRowProps {
  label: string
  localBase: string
  lanBase?: string | null
  path: string
  idPrefix: string
  onCopy: (url: string) => void
  copyLabel: string
  lanLabel: string
}

function EndpointRow({ label, localBase, lanBase, path, idPrefix, onCopy, copyLabel, lanLabel }: EndpointRowProps) {
  const localUrl = `${localBase}${path}`
  const lanUrl = lanBase ? `${lanBase}${path}` : null
  return (
    <div className="endpoint-row">
      <div className="endpoint-label">{label}</div>
      <div className="endpoint-box">
        <code id={`endpoint-${idPrefix}`}>{localUrl}</code>
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => void onCopy(localUrl)}>
          {copyLabel}
        </button>
      </div>
      {lanUrl && (
        <div className="endpoint-box endpoint-box-lan">
          <span className="endpoint-box-tag">{lanLabel}</span>
          <code id={`endpoint-${idPrefix}-lan`}>{lanUrl}</code>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => void onCopy(lanUrl)}>
            {copyLabel}
          </button>
        </div>
      )}
    </div>
  )
}

export function OverviewPage() {
  const { t } = useI18n()
  const gatewayBase = useAppStore((s) => s.gatewayBase.replace(/\/$/, ''))
  const status = useAppStore((s) => s.status)
  const setup = useSetupStore((s) => s.setup)
  const showToast = useAppStore((s) => s.showToast)
  const liveUptime = useLiveUptime()

  const lanBase = status?.lan_base_url?.replace(/\/$/, '') ?? null
  const listenLan = status?.listen_lan ?? setup?.gateway?.listen_lan

  const copyUrl = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url)
      showToast('toast.copied')
    } catch {
      showToast('toast.copyFail', false)
    }
  }

  const endpoints = [
    { id: 'openai', labelKey: 'overview.endpointOpenAi' as const, path: '/v1' },
    { id: 'responses', labelKey: 'overview.endpointResponses' as const, path: '/v1/responses' },
    { id: 'anthropic', labelKey: 'overview.endpointAnthropic' as const, path: '/anthropic' },
  ]

  return (
    <section className="page active" id="page-overview">
      <AgentQuickSetupCard />
      <CcSwitchExportCard />

      <div className="panel">
        <div className="panel-title">{t('overview.endpoint')}</div>
        <p className="endpoint-hint">{t('overview.endpointHint')}</p>
        {listenLan && !lanBase && (
          <p className="endpoint-hint endpoint-lan-unavailable">{t('overview.lanUnavailable')}</p>
        )}
        <div className="endpoint-list">
          {endpoints.map(({ id, labelKey, path }) => (
            <EndpointRow
              key={id}
              idPrefix={id}
              label={t(labelKey)}
              localBase={gatewayBase}
              lanBase={lanBase}
              path={path}
              onCopy={copyUrl}
              copyLabel={t('action.copy')}
              lanLabel={t('overview.endpointLan')}
            />
          ))}
        </div>
      </div>

      <div className="panel">
        <div className="panel-title">{t('gateway.statusPanel')}</div>
        <div className="stat-grid gateway-status-grid">
          <div className="stat-box">
            <div className="label">{t('gateway.runState')}</div>
            <div className="value" id="gw-stat-state">
              {status?.status === 'running' ? t('status.running') : status?.status === 'stopped' ? t('status.stopped') : '—'}
            </div>
          </div>
          <div className="stat-box">
            <div className="label">{t('gateway.version')}</div>
            <div className="value" id="gw-stat-version">{status?.version ?? '—'}</div>
          </div>
          <div className="stat-box">
            <div className="label">{t('gateway.uptime')}</div>
            <div className="value" id="gw-stat-uptime">
              {status?.status === 'running' ? formatUptime(liveUptime, t) : '—'}
            </div>
          </div>
          <div className="stat-box">
            <div className="label">{t('gateway.listenAddr')}</div>
            <div className="value" id="gw-stat-listen">{status?.listen ?? '—'}</div>
          </div>
        </div>
      </div>
    </section>
  )
}
