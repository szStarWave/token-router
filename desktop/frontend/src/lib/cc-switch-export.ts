import { useAppStore } from '../stores/appStore'
import { useSetupStore } from '../stores/setupStore'
import { resolveAgentContextWindow, resolveApiKey, type AgentKind } from './agent-quick-setup'

export type CcSwitchApp = 'claude' | 'codex' | 'openclaw' | 'gemini' | 'opencode' | 'hermes'

export const CC_SWITCH_RELEASES_URL = 'https://github.com/farion1231/cc-switch/releases'

const PROVIDER_NAME = 'Token Router'
const OPENCLAW_PROVIDER = 'token-router'
const OPENCLAW_MODEL_DISPLAY = 'Token Router Auto Route'
const OPENCLAW_CONTEXT_WINDOW = 1_000_000
const OPENCLAW_TIMEOUT_SECONDS = 300
const CODEX_PROVIDER = 'token_router'
const CODEX_PROVIDER_NAME = 'TokenRouter'
const OPENCODE_PROVIDER = 'token-router'
const OPENCODE_PROVIDER_NAME = 'Token Router'
const OPENCODE_MODEL_DISPLAY = 'Token Router Auto Route'
const CODEX_MODEL = 'token-router'
const DEFAULT_MODEL = 'auto'

const CC_SWITCH_AGENT_MAP: Record<CcSwitchApp, AgentKind> = {
  claude: 'claude-code',
  codex: 'codex',
  openclaw: 'openclaw',
  hermes: 'hermes',
  gemini: 'claude-code',
  opencode: 'opencode',
}

export interface CcSwitchExportParams {
  baseUrl: string
  apiKey: string
  model: string
  providerName?: string
  contextWindow?: number
}

function utf8ToBase64(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let binary = ''
  for (const byte of bytes) {
    binary += String.fromCharCode(byte)
  }
  return btoa(binary)
}

function buildCodexToml(baseUrl: string, model: string, apiKey: string, contextWindow?: number): string {
  const lines = [
    `model = "${model}"`,
    `model_provider = "${CODEX_PROVIDER}"`,
    `model_catalog_json = "token-router-model-catalog.json"`,
    `model_reasoning_effort = "medium"`,
    `disable_response_storage = true`,
  ]
  if (contextWindow != null && contextWindow > 0) {
    lines.push(`model_context_window = ${contextWindow}`)
  }
  lines.push(
    '',
    `[model_providers.${CODEX_PROVIDER}]`,
    `name = "${CODEX_PROVIDER_NAME}"`,
    `base_url = "${baseUrl}"`,
    `experimental_bearer_token = "${apiKey}"`,
    'wire_api = "responses"',
    'requires_openai_auth = true',
    '',
  )
  return lines.join('\n')
}

function buildOpenClawJson(baseUrl: string, apiKey: string): string {
  return JSON.stringify({
    models: {
      providers: {
        [OPENCLAW_PROVIDER]: {
          baseUrl,
          apiKey,
          timeoutSeconds: OPENCLAW_TIMEOUT_SECONDS,
          models: [{ id: DEFAULT_MODEL, name: OPENCLAW_MODEL_DISPLAY, contextWindow: OPENCLAW_CONTEXT_WINDOW }],
        },
      },
    },
    agents: {
      defaults: {
        model: {
          primary: `${OPENCLAW_PROVIDER}/${DEFAULT_MODEL}`,
        },
      },
    },
  })
}

function buildHermesJson(baseUrl: string, model: string, apiKey: string): string {
  return JSON.stringify({
    model: {
      default: model,
      provider: 'custom',
      base_url: baseUrl,
      api_key: apiKey,
    },
  })
}

function buildOpenCodeJson(baseUrl: string, model: string, apiKey: string): string {
  return JSON.stringify({
    $schema: 'https://opencode.ai/config.json',
    provider: {
      [OPENCODE_PROVIDER]: {
        npm: '@ai-sdk/openai-compatible',
        name: OPENCODE_PROVIDER_NAME,
        options: {
          baseURL: baseUrl,
          apiKey,
        },
        models: {
          [model]: {
            name: OPENCODE_MODEL_DISPLAY,
          },
        },
      },
    },
    model: `${OPENCODE_PROVIDER}/${model}`,
  })
}

function resolveModel(): string {
  const model = useSetupStore.getState().setup?.cloud?.model?.trim()
  return model && model.length > 0 ? model : DEFAULT_MODEL
}

function resolveEndpoint(app: CcSwitchApp, gatewayBase: string): string {
  const base = gatewayBase.replace(/\/$/, '')
  if (app === 'claude') return `${base}/anthropic`
  return `${base}/v1`
}

export function buildCcSwitchImportUrl(app: CcSwitchApp, opts: CcSwitchExportParams): string {
  const name = opts.providerName ?? PROVIDER_NAME
  const params = new URLSearchParams({
    resource: 'provider',
    app,
    name,
    notes: 'Imported from Token Router',
  })

  switch (app) {
    case 'claude':
      params.set('endpoint', opts.baseUrl)
      params.set('apiKey', opts.apiKey)
      params.set('model', opts.model)
      break
    case 'codex': {
      params.set('configFormat', 'toml')
      params.set(
        'config',
        utf8ToBase64(
          buildCodexToml(opts.baseUrl, opts.model, opts.apiKey, opts.contextWindow),
        ),
      )
      break
    }
    case 'openclaw': {
      params.set('configFormat', 'json')
      params.set('config', utf8ToBase64(buildOpenClawJson(opts.baseUrl, opts.apiKey)))
      break
    }
    case 'hermes': {
      params.set('configFormat', 'json')
      params.set('config', utf8ToBase64(buildHermesJson(opts.baseUrl, opts.model, opts.apiKey)))
      params.set('endpoint', opts.baseUrl)
      params.set('apiKey', opts.apiKey)
      params.set('model', opts.model)
      break
    }
    case 'opencode': {
      params.set('configFormat', 'json')
      params.set('config', utf8ToBase64(buildOpenCodeJson(opts.baseUrl, opts.model, opts.apiKey)))
      params.set('endpoint', opts.baseUrl)
      params.set('apiKey', opts.apiKey)
      params.set('model', opts.model)
      break
    }
    case 'gemini':
      params.set('endpoint', opts.baseUrl)
      params.set('apiKey', opts.apiKey)
      params.set('model', opts.model)
      break
  }

  return `ccswitch://v1/import?${params.toString()}`
}

export async function exportToCcSwitch(app: CcSwitchApp): Promise<string> {
  const gatewayBase = useAppStore.getState().gatewayBase
  const agentKind = CC_SWITCH_AGENT_MAP[app]
  const apiKey = await resolveApiKey(agentKind)
  if (!apiKey?.trim()) {
    throw new Error('missing_api_key')
  }

  const baseUrl = resolveEndpoint(app, gatewayBase)
  const model = app === 'codex' ? CODEX_MODEL : resolveModel()
  const contextWindow = app === 'codex' ? resolveAgentContextWindow() : undefined
  return buildCcSwitchImportUrl(app, { baseUrl, apiKey, model, contextWindow })
}

export function ccSwitchAppLabel(app: CcSwitchApp): string {
  switch (app) {
    case 'claude':
      return 'Claude Code'
    case 'codex':
      return 'Codex'
    case 'openclaw':
      return 'OpenClaw'
    case 'hermes':
      return 'Hermes Agent'
    case 'gemini':
      return 'Gemini CLI'
    case 'opencode':
      return 'OpenCode'
  }
}

export function ccSwitchExportErrorMessage(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  if (err && typeof err === 'object' && 'message' in err) return String((err as Error).message)
  return String(err)
}
