/**
 * 余额页（供应商模块）：查询厂商公开的 usage 接口，展示账户余额与订阅/限额。
 *
 * 诚实口径：
 * - 只展示厂商接口返回值；没有公开接口的厂商明确显示「未提供余额接口」；
 * - 金额保留接口原始字符串，不跨币种求和、不用本地账本冒充余额；
 * - 单个账号失败只影响该行，不伪造 0。
 */
import { Link } from '@tanstack/react-router'
import { useProviderUsageQuery, useRefreshProviderUsageMutation } from '../queries/admin'
import { EmptyState, ErrorState, Loading } from '../components/common/QueryState'
import type { AccountUsageInfo, BalanceInfo, UsageLimitInfo } from '../api/types'
import { formatRelativeTime } from './providers/shared'

const PLAN_LABEL: Record<string, string> = {
  free_tier: '免费层',
  pay_as_you_go: '按量付费',
  opencode_go: 'OpenCode Go',
}

const WINDOW_LABEL: Record<string, string> = {
  rolling: '滚动',
  weekly: '本周',
  monthly: '本月',
}

function planLabel(plan: string): string {
  return PLAN_LABEL[plan] ?? plan
}

function windowLabel(label: string): string {
  return WINDOW_LABEL[label] ?? label
}

function formatPercent(percent: number): string {
  const value = Number.isInteger(percent) ? String(percent) : percent.toFixed(1)
  return `${value}%`
}

function currencyPrefix(currency: string): string {
  if (currency === 'USD') return '$'
  if (currency === 'CNY') return '¥'
  return `${currency} `
}

function amount(currency: string, value: string | null | undefined): string {
  if (value === null || value === undefined || value === '') return '—'
  return `${currencyPrefix(currency)}${value}`
}

function balanceDetail(balance: BalanceInfo): string {
  const parts: string[] = []
  if (balance.used) parts.push(`已用 ${amount(balance.currency, balance.used)}`)
  if (balance.granted) parts.push(`赠送 ${amount(balance.currency, balance.granted)}`)
  if (balance.topped_up) parts.push(`充值 ${amount(balance.currency, balance.topped_up)}`)
  if (balance.total) parts.push(`总额 ${amount(balance.currency, balance.total)}`)
  return parts.join(' · ')
}

function limitText(limits: UsageLimitInfo): string {
  const parts: string[] = []
  if (limits.remaining || limits.limit) {
    parts.push(
      `剩余 ${amount(limits.currency, limits.remaining)} / ${amount(limits.currency, limits.limit)}`,
    )
  }
  if (limits.used) parts.push(`已用 ${amount(limits.currency, limits.used)}`)
  return parts.join(' · ')
}

export function QuotasPage() {
  const usage = useProviderUsageQuery()
  const refresh = useRefreshProviderUsageMutation()

  if (usage.isPending) return <Loading label="查询账户余额…" />
  if (usage.isError) {
    return <ErrorState message={usage.error.message} onRetry={() => usage.refetch()} />
  }

  const accounts = usage.data?.accounts ?? []
  const fetchedAt = accounts.reduce((latest, item) => Math.max(latest, item.fetched_at), 0)

  const groups = new Map<string, { name: string; accounts: AccountUsageInfo[] }>()
  for (const account of accounts) {
    const group = groups.get(account.provider_id)
    if (group) group.accounts.push(account)
    else {
      groups.set(account.provider_id, {
        name: account.provider_name,
        accounts: [account],
      })
    }
  }

  return (
    <div className="app-page flex-1">
      <div className="app-card-actions">
        <button
          type="button"
          className="oc-btn-default oc-btn-sm"
          onClick={() => refresh.mutate()}
          disabled={refresh.isPending || accounts.length === 0}
          title="立即重新查询厂商 usage 接口；后端缓存 60 秒，避免频繁请求。"
        >
          {refresh.isPending ? '查询中…' : '刷新'}
        </button>
        <span className="flex-1" />
        <span className="text-meta text-muted-foreground">
          {fetchedAt > 0 ? `上次查询 ${formatRelativeTime(fetchedAt)}` : ''}
        </span>
      </div>

      {refresh.isError && (
        <div className="app-banner app-banner-err">
          查询失败：{refresh.error.message}
        </div>
      )}

      {accounts.length === 0 ? (
        <EmptyState
          title="没有账户"
          hint="先在厂商与账号里添加一个账号。"
          action={
            <Link to="/providers" className="oc-btn-primary oc-btn-sm">
              去添加账号
            </Link>
          }
        />
      ) : (
        <div className="flex flex-col gap-4">
          {[...groups.entries()].map(([providerId, group]) => (
            <section key={providerId} className="app-card" aria-label={group.name}>
              <h3 className="text-ui-label font-medium">{group.name}</h3>
              <div className="flex flex-col gap-3 pt-2">
                {group.accounts.map((account) => (
                  <AccountRow key={account.account_id} account={account} />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  )
}

function AccountRow({ account }: { account: AccountUsageInfo }) {
  return (
    <div className="flex flex-col gap-1 border-t border-border pt-3 first:border-t-0 first:pt-0">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-ui-label font-medium">{account.account_label}</span>
        <code className="app-inline-code">…{account.fingerprint}</code>
        {!account.enabled && <span className="app-pill app-pill-warn">停用</span>}
        {account.status === 'ok' && account.plan && (
          <span className="app-pill">{planLabel(account.plan)}</span>
        )}
        <span className="ml-auto text-meta text-muted-foreground">
          {account.fetched_at > 0 ? formatRelativeTime(account.fetched_at) : ''}
        </span>
      </div>

      {account.status === 'unsupported' ? (
        <span
          className="text-meta text-muted-foreground"
          title={account.error ?? undefined}
        >
          未提供余额接口
          {account.error?.includes('管理员') ? '（可能需要管理员 key）' : ''}
        </span>
      ) : account.status === 'error' ? (
        <span className="text-meta text-status-error">查询失败：{account.error ?? '未知错误'}</span>
      ) : (
        <>
          {account.balances.length === 0 && account.windows.length === 0 && (
            <span className="text-meta text-muted-foreground">—</span>
          )}
          {account.balances.map((balance) => {
            const detail = balanceDetail(balance)
            return (
              <div key={balance.currency} className="flex flex-wrap items-baseline gap-2">
                <span className="text-ui-label font-medium">
                  {amount(balance.currency, balance.available)}
                </span>
                <span className="text-meta text-muted-foreground">
                  {balance.currency} 可用
                </span>
                {detail && (
                  <span className="text-meta text-muted-foreground">· {detail}</span>
                )}
              </div>
            )
          })}
          {account.windows.map((window) => (
            <div
              key={window.label}
              className="flex flex-wrap items-center gap-2 text-meta"
            >
              <span className="text-ui-label text-foreground">
                {windowLabel(window.label)}
              </span>
              <span className={window.limited ? 'text-status-error' : 'text-muted-foreground'}>
                {formatPercent(window.percent)}
              </span>
              {window.limited && <span className="app-pill app-pill-error">已限流</span>}
              {window.resets_at !== undefined && window.resets_at !== null && (
                <span className="text-muted-foreground">
                  重置于 {formatRelativeTime(window.resets_at)}
                </span>
              )}
            </div>
          ))}
          {account.limits && (
            <span className="text-meta text-muted-foreground">
              键限额：{limitText(account.limits)}
            </span>
          )}
        </>
      )}
    </div>
  )
}
