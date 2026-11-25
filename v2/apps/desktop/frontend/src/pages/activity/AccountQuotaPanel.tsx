/**
 * 账号与配额（活动页底部）：按 provider 分组的账号清单 + 每个账号的配额观测。
 *
 * 凭据只显示引用与指纹后 4 位（明文永远不下发到前端）；配额没有观测就写
 * 「未观测」，不拿估算值冒充上游额度。
 */
import type { AccountInfo, ProviderInfo, QuotaWindowInfo } from '../../api/types'
import { formatCompactNum } from '../../lib/format-number'
import { fmtRelative, quotaState, quotaStateLabel, quotaStatePill } from './shared'

export interface AccountQuotaPanelProps {
  providers: ProviderInfo[]
  accounts: AccountInfo[]
  quotas: QuotaWindowInfo[]
  nowMs: number
}

export function AccountQuotaPanel({
  providers,
  accounts,
  quotas,
  nowMs,
}: AccountQuotaPanelProps) {
  if (providers.length === 0) {
    return <p className="text-meta text-muted-foreground">还没有供应商。</p>
  }

  return (
    <div className="flex flex-col gap-4">
      {providers.map((provider) => {
        const providerAccounts = accounts.filter((item) => item.provider_id === provider.id)
        return (
          <div key={provider.id} className="flex flex-col gap-2">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-ui-label font-semibold">{provider.display_name}</span>
              <span className="app-pill">{provider.kind}</span>
              {provider.enabled ? (
                <span className="app-pill app-pill-ok">已启用</span>
              ) : (
                <span className="app-pill app-pill-warn">已停用</span>
              )}
            </div>
            {providerAccounts.length === 0 ? (
              <p className="text-meta text-muted-foreground">还没有账号。</p>
            ) : (
              providerAccounts.map((account) => {
                const accountQuotas = quotas.filter((item) => item.account_id === account.id)
                return (
                  <div key={account.id} className="app-account-row">
                    <div className="flex items-center gap-2 flex-wrap min-w-0">
                      <span className="text-ui-label truncate">{account.label}</span>
                      {account.enabled ? (
                        <span className="app-pill app-pill-ok">启用</span>
                      ) : (
                        <span className="app-pill app-pill-warn">停用</span>
                      )}
                      <span className="font-mono text-meta text-muted-foreground">
                        尾号 {account.fingerprint || '—'}
                      </span>
                    </div>
                    <div className="flex items-center gap-2 flex-wrap text-meta text-muted-foreground">
                      <span className="font-mono">{account.credential_ref}</span>
                      {account.proxy_url && <span>代理 {account.proxy_url}</span>}
                      {account.extra_headers && account.extra_headers.length > 0 && (
                        <span>{account.extra_headers.length} 个自定义 header</span>
                      )}
                    </div>
                    {accountQuotas.length === 0 ? (
                      <p className="text-micro text-muted-foreground">还没有配额观测。</p>
                    ) : (
                      <div className="flex flex-col gap-1">
                        {accountQuotas.map((quota) => {
                          const state = quotaState(quota)
                          return (
                            <div key={quota.candidate_id} className="flex items-center gap-2 flex-wrap">
                              <span className="font-mono text-meta truncate">
                                {quota.candidate_id}
                              </span>
                              <span className={quotaStatePill(state)}>{quotaStateLabel(state)}</span>
                              {quota.remaining !== null && quota.limit !== null && (
                                <span className="text-meta text-muted-foreground tabular-nums">
                                  {formatCompactNum(quota.remaining)} / {formatCompactNum(quota.limit)}
                                </span>
                              )}
                              {quota.reset_at && (
                                <span className="text-meta text-muted-foreground">
                                  {fmtRelative(quota.reset_at, nowMs)}重置
                                </span>
                              )}
                              {quota.consecutive_429 > 0 && (
                                <span className="app-pill app-pill-error">
                                  429 × {quota.consecutive_429}
                                </span>
                              )}
                            </div>
                          )
                        })}
                      </div>
                    )}
                  </div>
                )
              })
            )}
          </div>
        )
      })}
    </div>
  )
}
