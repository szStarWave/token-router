/**
 * 网关页：客户端怎么连、模型有哪些、最近调用发生了什么。
 *
 * 形态参照 Magpie 的「模型 / 网关」页：
 *   1. 网关（状态 + 局域网共享 + 接入配置：协议 / Base URL / 密钥 / 模型 / 示例）
 *   2. 模型（可搜索的模型清单，可复制 id）
 *   3. 最近调用（账本流水）
 *   4. 设置（网关密钥与配置生效情况）
 *
 * 口径：地址来自运行中的 status.listen；模型来自运行中的配置；
 * 密钥明文只在创建时出现一次，这里只给名字与环境变量写法，不回显明文。
 */
import { useEffect, useMemo, useState } from 'react'
import { Link } from '@tanstack/react-router'
import {
  useConfigQuery,
  useRequestsQuery,
  useStatusQuery,
  useTokensQuery,
} from '../queries/admin'
import { QueryState } from '../components/common/QueryState'
import { Icon } from '../components/common/Icon'
import { useAppStore } from '../stores/appStore'
import { useConfigDraftStore } from '../stores/configDraftStore'
import { usePrefsStore } from '../stores/prefsStore'
import { draftFromDocument } from '../lib/config-edit'
import { formatCompactNum } from '../lib/format-number'
import { statusPillClass } from '../lib/status'
import { hostLanAddresses } from '../lib/tauri'
import { fmtClock, fmtDuration, formatModelLabel, protocolLabel } from './activity/shared'

type AccessView = 'access' | 'status'
type ShellKind = 'powershell' | 'bash' | 'node' | 'python'
type ShareTarget = 'local' | 'lan'

const ENDPOINTS = [
  { path: '/v1', label: 'OpenAI 兼容', hint: 'Chat Completions 客户端' },
  { path: '/responses', label: 'Responses', hint: '原生 Responses API 客户端' },
  { path: '/anthropic', label: 'Anthropic', hint: 'Anthropic Messages 协议客户端' },
] as const

const SHELLS: Array<{ key: ShellKind; label: string }> = [
  { key: 'powershell', label: 'PowerShell' },
  { key: 'bash', label: 'bash' },
  { key: 'node', label: 'Node' },
  { key: 'python', label: 'Python' },
]

const CALL_LIMIT = 20

/** listen → 端口。兼容 IPv6（`[::1]:17621`），不再用 split(':')[1]。 */
function portOf(listen: string | undefined): string {
  if (!listen) return '17621'
  const bracketed = /\[[^\]]*\]:(\d+)$/.exec(listen)
  if (bracketed?.[1]) return bracketed[1]
  const index = listen.lastIndexOf(':')
  return index >= 0 ? listen.slice(index + 1) : '17621'
}

export function GatewayPage() {
  const statusQuery = useStatusQuery()
  const configQuery = useConfigQuery()
  const tokensQuery = useTokensQuery()
  const requestsQuery = useRequestsQuery({ limit: 200 })
  const showToast = useAppStore((s) => s.showToast)
  const restartPending = useConfigDraftStore((s) => s.restartPending)
  const modelNamePref = usePrefsStore((s) => s.modelName)

  const [view, setView] = useState<AccessView>('access')
  const [shareTarget, setShareTarget] = useState<ShareTarget>('local')
  const [shell, setShell] = useState<ShellKind>('powershell')
  const [modelQuery, setModelQuery] = useState('')
  const [showAllCalls, setShowAllCalls] = useState(false)
  const [lanAddresses, setLanAddresses] = useState<string[]>([])

  const status = statusQuery.data
  const config = configQuery.data
  // 磁盘配置与运行中 digest 不一致：模型清单可能还没生效。
  const configPendingRestart =
    config !== undefined &&
    status !== undefined &&
    config.revision !== status.config_digest

  useEffect(() => {
    void hostLanAddresses()
      .then(setLanAddresses)
      .catch(() => setLanAddresses([]))
  }, [])

  const draft = useMemo(
    () => (config ? draftFromDocument(config) : null),
    [config],
  )

  const baseUrl = status ? `http://${status.listen}` : null
  const openAiBase = baseUrl ? `${baseUrl}/v1` : 'http://<listen>/v1'

  const tokens = tokensQuery.data?.items ?? []
  const activeTokens = tokens.filter((token) => token.revoked_at === null)

  // 模型清单：路由组 id（客户端可填）+ 池内成员的客户端 id（`组/成员`，
  // 显示形态跟设置页「模型名显示」：完整 / 去掉组前缀）。
  const models = useMemo(() => {
    if (!draft) return []
    const keyword = modelQuery.trim().toLowerCase()
    return draft.virtualModels
      .map((group) => {
        const candidates = group.candidate_pool
          .map((id) => draft.candidates.find((candidate) => candidate.id === id))
          .filter((candidate) => candidate !== undefined)
        const haystack = [
          group.id,
          group.name,
          ...candidates.map((candidate) => `${candidate.model} ${candidate.provider}`),
        ]
          .join(' ')
          .toLowerCase()
        return { group, candidates, haystack }
      })
      .filter((item) => (keyword ? item.haystack.includes(keyword) : true))
  }, [draft, modelQuery])

  const calls = requestsQuery.data?.items ?? []
  const shownCalls = showAllCalls ? calls : calls.slice(0, CALL_LIMIT)

  const copy = async (text: string, label: string) => {
    try {
      if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable')
      await navigator.clipboard.writeText(text)
      showToast(label ? `已复制 ${label}` : 'toast.copied')
    } catch {
      showToast('复制失败，请手动选择内容复制', false)
    }
  }

  const copyAllModelIds = async () => {
    const ids = models.map((item) => item.group.id).join('\n')
    if (!ids) {
      showToast('没有可复制的模型 id', false)
      return
    }
    await copy(ids, '全部模型 id')
  }

  const shellSnippet = (() => {
    switch (shell) {
      case 'powershell':
        return `$env:OPENAI_BASE_URL="${openAiBase}"\n$env:OPENAI_API_KEY="<网关 token>"`
      case 'bash':
        return `export OPENAI_BASE_URL="${openAiBase}"\nexport OPENAI_API_KEY="<网关 token>"`
      case 'node':
        return `process.env.OPENAI_BASE_URL = '${openAiBase}'\nprocess.env.OPENAI_API_KEY = '<网关 token>'`
      default:
        return `os.environ['OPENAI_BASE_URL'] = '${openAiBase}'\nos.environ['OPENAI_API_KEY'] = '<网关 token>'`
    }
  })()

  const shareUrl =
    shareTarget === 'local'
      ? baseUrl ?? 'http://<listen>'
      : lanAddresses.length > 0
        ? `http://${lanAddresses[0]}:${portOf(status?.listen)}`
        : '未探测到局域网地址'

  return (
    <div className="app-page">
      {/* 1. 网关 */}
      <section className="app-card" aria-label="网关">
        <div className="flex items-center gap-3 flex-wrap">
          <span className="app-card-title">网关</span>
          <span className={status ? 'app-pill app-pill-ok' : 'app-pill'}>
            {status ? '运行中' : '未连接'}
          </span>
          <span className="text-meta text-muted-foreground">
            {models.length} 个模型入口 · {activeTokens.length} 个网关密钥
          </span>
          <button
            type="button"
            className="oc-btn-default oc-btn-sm ml-auto"
            onClick={() => {
              void statusQuery.refetch()
              void configQuery.refetch()
              void tokensQuery.refetch()
              void requestsQuery.refetch()
            }}
          >
            <Icon name="refresh" />
            更新
          </button>
          <div className="oc-seg" role="group" aria-label="网关视图">
            <button
              type="button"
              className={view === 'access' ? 'oc-seg-item-active' : 'oc-seg-item'}
              aria-pressed={view === 'access'}
              onClick={() => setView('access')}
            >
              接入
            </button>
            <button
              type="button"
              className={view === 'status' ? 'oc-seg-item-active' : 'oc-seg-item'}
              aria-pressed={view === 'status'}
              onClick={() => setView('status')}
            >
              连接状态
            </button>
          </div>
        </div>

        {view === 'status' ? (
          <QueryState
            loading={statusQuery.isLoading}
            error={statusQuery.error}
            onRetry={() => void statusQuery.refetch()}
          >
            <dl className="app-kv">
              <div className="app-kv-item">
                <dt className="app-kv-key">版本</dt>
                <dd className="app-kv-value">{status?.version ?? '—'}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">监听地址</dt>
                <dd className="app-kv-value font-mono">{status?.listen ?? '—'}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">启动时间</dt>
                <dd className="app-kv-value">
                  {status ? new Date(status.started_at).toLocaleString() : '—'}
                </dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">累计请求</dt>
                <dd className="app-kv-value tabular-nums">
                  {status?.requests_total.toLocaleString() ?? '—'}
                </dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">默认虚拟模型</dt>
                <dd className="app-kv-value font-mono">{status?.default_virtual_model || '—'}</dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">配置版本</dt>
                <dd className="app-kv-value font-mono" title={status?.config_digest}>
                  {status?.config_digest ? `${status.config_digest.slice(0, 8)} · 运行中` : '—'}
                </dd>
              </div>
              <div className="app-kv-item">
                <dt className="app-kv-key">待发布入口</dt>
                <dd className="app-kv-value">
                  {restartPending ? '监听/鉴权变更（重启后生效）' : '无'}
                </dd>
              </div>
            </dl>
          </QueryState>
        ) : (
          <>
            {/* 局域网共享 */}
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-ui-label font-semibold">局域网共享</span>
                <div className="oc-seg ml-auto" role="group" aria-label="地址来源">
                  <button
                    type="button"
                    className={shareTarget === 'local' ? 'oc-seg-item-active' : 'oc-seg-item'}
                    aria-pressed={shareTarget === 'local'}
                    onClick={() => setShareTarget('local')}
                  >
                    本机
                  </button>
                  <button
                    type="button"
                    className={shareTarget === 'lan' ? 'oc-seg-item-active' : 'oc-seg-item'}
                    aria-pressed={shareTarget === 'lan'}
                    onClick={() => setShareTarget('lan')}
                  >
                    网关
                  </button>
                </div>
              </div>
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-meta text-muted-foreground">
                  {shareTarget === 'local' ? '本机' : '局域网'}
                </span>
                <code className="app-inline-code break-all">{shareUrl}</code>
                {shareTarget === 'local' && (
                  <button
                    type="button"
                    className="oc-btn-default oc-btn-sm"
                    onClick={() => void copy(shareUrl, '地址')}
                  >
                    <Icon name="copy" />
                    复制
                  </button>
                )}
              </div>
            </div>

            {/* 接入 */}
            <div className="flex flex-col gap-3">
              <span className="text-ui-label font-semibold">接入</span>

              <div className="app-gateway-row">
                <span className="app-gateway-key">API</span>
                <div className="flex items-center gap-2 flex-wrap">
                  {ENDPOINTS.map((endpoint) => (
                    <span key={endpoint.path} className="app-pill" title={endpoint.hint}>
                      {endpoint.label}
                    </span>
                  ))}
                </div>
              </div>

              <div className="app-gateway-row">
                <span className="app-gateway-key">Base URL</span>
                <div className="flex items-center gap-2 flex-wrap min-w-0">
                  <code className="app-inline-code break-all">{openAiBase}</code>
                  <button
                    type="button"
                    className="oc-btn-default oc-btn-sm"
                    onClick={() => void copy(openAiBase, 'Base URL')}
                  >
                    <Icon name="copy" />
                    复制
                  </button>
                </div>
              </div>

              <div className="app-gateway-row">
                <span className="app-gateway-key">API 密钥</span>
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="font-mono text-ui-label">
                    {activeTokens.length > 0
                      ? activeTokens.map((token) => token.name).join('、')
                      : '尚未创建网关 token'}
                  </span>
                </div>
              </div>

              <div className="app-gateway-row">
                <span className="app-gateway-key">模型</span>
                <div className="flex items-center gap-2 flex-wrap min-w-0">
                  {models.length === 0 ? (
                    <span className="text-meta text-muted-foreground">—</span>
                  ) : (
                    models.slice(0, 8).map((item) => (
                      <button
                        key={item.group.id}
                        type="button"
                        className="app-pill app-pill-clickable"
                        onClick={() => void copy(item.group.id, item.group.id)}
                      >
                        {item.group.id}
                        <Icon name="copy" className="size-3" />
                      </button>
                    ))
                  )}
                </div>
              </div>

              <div className="app-gateway-row">
                <span className="app-gateway-key">示例</span>
                <div className="oc-seg" role="group" aria-label="示例语言">
                  {SHELLS.map((item) => (
                    <button
                      key={item.key}
                      type="button"
                      className={shell === item.key ? 'oc-seg-item-active' : 'oc-seg-item'}
                      aria-pressed={shell === item.key}
                      onClick={() => setShell(item.key)}
                    >
                      {item.label}
                    </button>
                  ))}
                </div>
              </div>
              <pre className="app-code-block">{shellSnippet}</pre>
            </div>
          </>
        )}
      </section>

      {/* 2. 模型 */}
      <section className="app-card" aria-label="模型">
        <div className="flex items-center gap-2 flex-wrap">
          <span className="app-card-title">模型 ({models.length})</span>
          <input
            className="oc-field app-field-light app-setting-search"
            value={modelQuery}
            placeholder="选模型…"
            aria-label="筛选模型"
            onChange={(e) => setModelQuery(e.target.value)}
          />
          <button
            type="button"
            className="oc-btn-default oc-btn-sm"
            onClick={() => void copyAllModelIds()}
          >
            <Icon name="copy" />
            复制全部 ID
          </button>
        </div>
        {configPendingRestart && (
          <div className="app-banner app-banner-warn text-meta">
            配置已保存，网关尚未重启；运行时仍是旧模型。
          </div>
        )}
        <QueryState
          loading={configQuery.isLoading}
          error={configQuery.error}
          onRetry={() => void configQuery.refetch()}
        >
          {models.length === 0 ? (
            <p className="text-meta text-muted-foreground">
              {modelQuery ? '没有匹配的模型。' : '还没有模型入口。'}
            </p>
          ) : (
            <div className="flex flex-col">
              {models.map((item) => (
                <div key={item.group.id} className="app-model-row">
                  <div className="flex items-center gap-2 flex-wrap min-w-0">
                    <span className="font-mono text-ui-label font-semibold truncate">
                      {item.group.id}
                    </span>
                    <span className="app-pill">{item.candidates.length} 候选</span>
                    {item.group.context_window && (
                      <span className="app-pill">
                        上下文 {formatCompactNum(item.group.context_window)}
                      </span>
                    )}
                    <button
                      type="button"
                      className="oc-btn-default oc-btn-sm ml-auto"
                      aria-label={`复制模型 id ${item.group.id}`}
                      onClick={() => void copy(item.group.id, item.group.id)}
                    >
                      <Icon name="copy" />
                    </button>
                  </div>
                  {item.candidates.length > 0 && (
                    <div className="flex items-center gap-2 flex-wrap text-meta text-muted-foreground pl-1">
                      {item.candidates.map((candidate) => (
                        <span key={candidate.id} className="font-mono">
                          {formatModelLabel(`${item.group.id}/${candidate.model}`, modelNamePref)}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </QueryState>
      </section>

      {/* 3. 最近调用 */}
      <section className="app-card" aria-label="最近调用">
        <span className="app-card-title">最近调用</span>
        <QueryState
          loading={requestsQuery.isLoading}
          error={requestsQuery.error}
          empty={calls.length === 0}
          onRetry={() => void requestsQuery.refetch()}
          emptyTitle="还没有调用记录"
        >
          <table className="app-table">
            <thead>
              <tr>
                <th>时间</th>
                <th>模型</th>
                <th>协议</th>
                <th>结果</th>
                <th>tokens</th>
                <th>首字节</th>
              </tr>
            </thead>
            <tbody>
              {shownCalls.map((row) => (
                <tr key={row.request_id}>
                  <td className="font-mono">{fmtClock(row.created_at)}</td>
                  <td className="font-mono">
                    {row.selection_model} → {row.dispatch_model ?? '—'}
                  </td>
                  <td>{protocolLabel(row.ingress_protocol)}</td>
                  <td>
                    <span className={statusPillClass(row.outcome)}>{row.outcome}</span>
                  </td>
                  <td className="tabular-nums">
                    {formatCompactNum(row.tokens.input + row.tokens.output)}
                  </td>
                  <td>{fmtDuration(row.ttft_ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {calls.length > shownCalls.length && (
            <button
              type="button"
              className="oc-btn-default oc-btn-sm self-start"
              onClick={() => setShowAllCalls(true)}
            >
              显示其余 {calls.length - shownCalls.length} 条
            </button>
          )}
        </QueryState>
      </section>

      {/* 4. 设置 */}
      <section className="app-card" aria-label="设置">
        <span className="app-card-title">设置</span>
        <div className="app-gateway-row">
          <span className="app-gateway-key">网关密钥</span>
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-meta">
              {activeTokens.length} 个可用 / 共 {tokens.length} 个
            </span>
            <Link to="/tokens" className="oc-btn-default oc-btn-sm">
              <Icon name="nav-tokens" className="size-3.5" />
              管理密钥
            </Link>
          </div>
        </div>
        <div className="app-gateway-row">
          <span className="app-gateway-key">鉴权与监听</span>
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-meta text-muted-foreground">
              模式 {draft?.server.auth_mode ?? '—'} · 监听 {draft?.server.listen ?? '—'}
            </span>
            <Link to="/settings" className="oc-btn-default oc-btn-sm">
              <Icon name="nav-settings" className="size-3.5" />
              设置
            </Link>
            <Link to="/diagnostics" className="oc-btn-default oc-btn-sm">
              <Icon name="nav-diagnostics" className="size-3.5" />
              诊断
            </Link>
          </div>
        </div>
      </section>
    </div>
  )
}
