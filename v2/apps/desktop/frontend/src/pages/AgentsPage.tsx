import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { apiFetch } from '../lib/gateway'
import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { useI18n } from '../hooks/useI18n'
import { queryKeys } from '../queries/keys'
import type { UpstreamSetupView } from '../types/gateway'

export function AgentsPage() {
  const { t } = useI18n()
  const connected = useAppStore((s) => s.connected)
  const showToast = useAppStore((s) => s.showToast)
  const stats = useAppStore((s) => s.stats)
  const setSetup = useSetupStore((s) => s.setSetup)
  const qc = useQueryClient()
  const [agentId, setAgentId] = useState('')

  const budgets = stats?.agent_budgets ?? []

  const loadSetup = async () => {
    if (!connected) {
      showToast('conn.offline', false)
      return
    }
    try {
      const path = agentId.trim()
        ? `/v1/admin/setup?agent_id=${encodeURIComponent(agentId.trim())}`
        : '/v1/admin/setup'
      const view = await apiFetch<UpstreamSetupView>(path)
      setSetup(view)
      void qc.invalidateQueries({ queryKey: queryKeys.gatewaySetup(agentId.trim() || undefined) })
      showToast('toast.setupLoaded')
    } catch (e) {
      showToast('toast.loadFail', false, { msg: e instanceof Error ? e.message : String(e) })
    }
  }

  return (
    <section className="page active" id="page-agents">
      <div className="panel">
        <div className="panel-title">{t('agents.title')}</div>
        <div className="form-row" style={{ marginBottom: 16 }}>
          <div>
            <label>{t('field.agentId')}</label>
            <input id="agent_id" placeholder={t('ph.agentId')} value={agentId} onChange={(e) => setAgentId(e.target.value)} />
          </div>
          <div style={{ display: 'flex', alignItems: 'flex-end' }}>
            <button type="button" className="btn btn-ghost" onClick={() => void loadSetup()}>
              {t('action.loadAgent')}
            </button>
          </div>
        </div>
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{t('col.agent')}</th>
                <th>{t('col.cloudBudget')}</th>
                <th>{t('col.tokensUsed')}</th>
                <th>{t('col.usage')}</th>
              </tr>
            </thead>
            <tbody id="agent-table">
              {!budgets.length ? (
                <tr><td colSpan={4} style={{ color: 'var(--default-400)' }}>{t('agents.empty')}</td></tr>
              ) : (
                budgets.map((row) => {
                  const limit = row.budget_limit
                  const used = row.tokens_used
                  const pct = limit && limit > 0 ? Math.round((used / limit) * 100) : null
                  return (
                    <tr key={row.agent_id}>
                      <td><code>{row.agent_id}</code></td>
                      <td>{limit == null ? t('agents.unlimited') : limit.toLocaleString()}</td>
                      <td className="num">{used.toLocaleString()}</td>
                      <td className="num">{pct != null ? `${pct}%` : '—'}</td>
                    </tr>
                  )
                })
              )}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  )
}
