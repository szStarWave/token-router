/**
 * Onboarding（§7.3）：Agent 接入入口。
 * M5 落地 host detect/plan/apply/verify 的完整 UI；本页先提供
 * 8 个 Agent 的接入状态占位与 tr-cli 指引。
 */
import { useQuery } from '@tanstack/react-query'
import { api } from '../api/client'
import type { ConfigDocument } from '../api/types'
import { isTauri } from '../lib/tauri'

const AGENTS = [
  { id: 'openclaw', label: 'OpenClaw', config: '~/.openclaw/openclaw.json' },
  { id: 'hermes', label: 'Hermes', config: '~/.hermes/config.yaml' },
  { id: 'hermes-flash', label: 'Hermes Flash', config: '~/.hermes-flash/config.yaml' },
  { id: 'claude-code', label: 'Claude Code', config: '~/.claude/settings.json' },
  { id: 'codex', label: 'Codex', config: '~/.codex/config.toml' },
  { id: 'opencode', label: 'OpenCode', config: '~/.config/opencode/opencode.json' },
  { id: 'codebuddy', label: 'CodeBuddy', config: '~/.codebuddy/models.json' },
  { id: 'workbuddy', label: 'WorkBuddy', config: '~/.workbuddy/models.json' },
] as const

export function OnboardingPage() {
  const configQuery = useQuery({
    queryKey: ['admin', 'config'],
    queryFn: () => api.config(),
    retry: 1,
  })
  const gatewayUrl = configQuery.data
    ? `http://${(configQuery.data.document as Record<string, Record<string, unknown>>)?.server?.listen ?? '127.0.0.1:17621'}`
    : 'http://127.0.0.1:17621'

  return (
    <div className="page">
      <section className="card" aria-label="接入说明">
        <h2>Agent 接入</h2>
        <p>
          v2 网关地址（数据面）：<code className="mono">{gatewayUrl}</code>
          {' '}· 模型：<code className="mono">auto</code>
        </p>
        <p className="text-dim">
          Agent 只需要 <code>inference</code> 作用域的 token（Token 页创建）；
          绝不要把 admin token 写进 Agent 配置。
        </p>
        {isTauri() && (
          <p className="text-dim">
            一键接入（detect → plan → apply → verify）将在 M5 提供；
            当前可用 CLI：<code>tr-cli admin token-create &lt;name&gt; --scopes inference</code>。
          </p>
        )}
      </section>

      <section className="card" aria-label="Agent 列表">
        <h2>支持的 Agent</h2>
        <table className="table">
          <thead>
            <tr><th>Agent</th><th>配置文件</th></tr>
          </thead>
          <tbody>
            {AGENTS.map((agent) => (
              <tr key={agent.id}>
                <td>{agent.label}</td>
                <td className="mono text-dim">{agent.config}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="card" aria-label="手动接入示例">
        <h2>手动接入示例（OpenAI 兼容协议）</h2>
        <pre className="pre-json">{`base_url = "${gatewayUrl}/v1"
api_key = "<inference token>"
model = "auto"`}</pre>
        <p className="text-dim">
          Claude Code 协议走 <code className="mono">{gatewayUrl}/anthropic</code>；
          Codex (responses) 走 <code className="mono">{gatewayUrl}/v1</code> + wire_api=responses。
        </p>
      </section>
    </div>
  )
}

// ConfigDocument 仅用于类型引用。
export type { ConfigDocument }
