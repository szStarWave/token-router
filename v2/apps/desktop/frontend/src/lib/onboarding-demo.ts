import { create } from 'zustand'
import type { StatsSnapshot, GatewayStatus } from '../types/gateway'
import type { HerdsmanModel } from '../stores/edgeStore'
import type { CloudModel } from '../lib/flowy/api'
import type { RoutingLogEntry } from './routing-log'

export const DEMO_SAVED_POINTS = 100000

export const DEMO_GATEWAY_STATUS: GatewayStatus = {
  service: 'gateway',
  status: 'running',
  listen: '127.0.0.1:8080',
  version: '1.0.0',
  uptime_secs: 3600,
  edge_configured: true,
  cloud_configured: true,
}

export function buildDemoStats(scope: 'session' | 'global' = 'session'): StatsSnapshot {
  return {
    scope,
    requests_total: 2847,
    routing: {
      edge: 1532,
      cloud: 947,
      cascade: 368,
      edge_pct: 53.8,
      cloud_pct: 33.3,
      cascade_pct: 12.9,
    },
    cascade: { edge_ok: 248, fallback_to_cloud: 120 },
    tokens: { edge_input: 1420000, edge_output: 310000, cloud_input: 2890000, cloud_output: 720000 },
    token_breakdown: {
      edge: { input: 1420000, output: 310000, cached: 380000, max_input: 128000, max_output: 64000 },
      cloud: { input: 2890000, output: 720000, cached: 150000, max_input: 256000, max_output: 128000 },
      total: { input: 4310000, output: 1030000, cached: 530000 },
      edge_share_pct: 32.9,
      cloud_share_pct: 67.1,
    },
    latency: {
      avg_request_ms: 1240,
      avg_ttft_ms: 320,
      avg_tps: 18.5,
      edge_tps: 24.2,
      cloud_tps: 12.8,
      p95_ms: 4200,
      p99_ms: 8900,
    },
    model_stats: [
      { tier: 'edge', model: 'herdsman-llama-3.2-3b', input: 980000, output: 210000, cached: 320000 },
      { tier: 'edge', model: 'herdsman-qwen-2.5-7b', input: 440000, output: 100000, cached: 60000 },
      { tier: 'cloud', model: 'gpt-4o', input: 2010000, output: 510000, cached: 100000 },
      { tier: 'cloud', model: 'claude-3.5-sonnet', input: 880000, output: 210000, cached: 50000 },
    ],
    step_kinds: { casual: 1420, work: 860, plan: 320, tool_use: 247 },
    agent_budgets: [],
  }
}

export const DEMO_EDGE_MODELS: HerdsmanModel[] = [
  { id: 'llama-3.2-3b', name: 'LLaMA 3.2 3B', endpoint: 'http://127.0.0.1:11434/v1', context_window: 128000 },
  { id: 'qwen-2.5-7b', name: 'Qwen 2.5 7B', endpoint: 'http://127.0.0.1:11434/v1', context_window: 32768 },
  { id: 'deepseek-coder-1.3b', name: 'DeepSeek Coder 1.3B', endpoint: 'http://127.0.0.1:11434/v1', context_window: 16384 },
]

export const DEMO_EDGE_SELECTED_KEY = 'herdsman:llama-3.2-3b'

export const DEMO_CLOUD_MODELS: CloudModel[] = [
  { id: 'gpt-4o', name: 'GPT-4o', context_window: 128000 },
  { id: 'claude-3.5-sonnet', name: 'Claude 3.5 Sonnet', context_window: 200000 },
  { id: 'gpt-4o-mini', name: 'GPT-4o mini', context_window: 128000 },
]

export const DEMO_CLOUD_SELECTED_KEY = 'flowy:gpt-4o'

export const DEMO_ROUTING_LOGS: RoutingLogEntry[] = [
  { id: 1001, timestamp: '2026-07-21T10:30:00Z', timeLabel: '10:30:00', route: 'edge', stepKind: 'casual', model: 'herdsman-llama-3.2-3b', userPreview: 'What is the capital of France?', hasUserPreview: true, reasonCodes: ['GATE_DEFAULT'], difficulty: 0.12, raw: '' },
  { id: 1002, timestamp: '2026-07-21T10:30:05Z', timeLabel: '10:30:05', route: 'cloud', stepKind: 'work', model: 'gpt-4o', servedModel: 'gpt-4o', userPreview: 'Write a comprehensive analysis of quantum computing', hasUserPreview: true, reasonCodes: ['PLAN_COMPLEX_TASK', 'DIFFICULTY_0.78'], difficulty: 0.78, raw: '' },
  { id: 1003, timestamp: '2026-07-21T10:30:12Z', timeLabel: '10:30:12', route: 'cascade', stepKind: 'plan', model: 'herdsman-llama-3.2-3b', servedModel: 'gpt-4o', userPreview: 'Debug this Python code and explain the fix', hasUserPreview: true, reasonCodes: ['CASUAL_EDGE_FALLBACK', 'DIFFICULTY_0.65'], difficulty: 0.65, raw: '' },
  { id: 1004, timestamp: '2026-07-21T10:30:18Z', timeLabel: '10:30:18', route: 'edge', stepKind: 'casual', model: 'herdsman-qwen-2.5-7b', userPreview: 'Translate "hello" to Spanish', hasUserPreview: true, reasonCodes: ['GATE_DEFAULT'], difficulty: 0.05, raw: '' },
  { id: 1005, timestamp: '2026-07-21T10:30:25Z', timeLabel: '10:30:25', route: 'cloud', stepKind: 'tool_use', model: 'gpt-4o', servedModel: 'gpt-4o', userPreview: 'Search the web for latest AI news and summarize', hasUserPreview: true, reasonCodes: ['TOOL_ERROR_STREAK', 'DIFFICULTY_0.91'], difficulty: 0.91, raw: '' },
]

interface OnboardingDemoState {
  active: boolean
  stats: StatsSnapshot | null
  globalStats: StatsSnapshot | null
  sessionSavedPoints: number | null
  globalSavedPoints: number | null
  status: GatewayStatus | null
  edgeModels: HerdsmanModel[]
  edgeSelectedKey: string | null
  cloudModels: CloudModel[]
  cloudSelectedKey: string | null
  routingLogs: RoutingLogEntry[]
  enable: () => void
  clear: () => void
}

export const useOnboardingDemo = create<OnboardingDemoState>((set) => ({
  active: false,
  stats: null,
  globalStats: null,
  sessionSavedPoints: null,
  globalSavedPoints: null,
  status: null,
  edgeModels: [],
  edgeSelectedKey: null,
  cloudModels: [],
  cloudSelectedKey: null,
  routingLogs: [],
  enable: () => set({
    active: true,
    stats: buildDemoStats('session'),
    globalStats: buildDemoStats('global'),
    sessionSavedPoints: DEMO_SAVED_POINTS,
    globalSavedPoints: DEMO_SAVED_POINTS,
    status: DEMO_GATEWAY_STATUS,
    edgeModels: DEMO_EDGE_MODELS,
    edgeSelectedKey: DEMO_EDGE_SELECTED_KEY,
    cloudModels: DEMO_CLOUD_MODELS,
    cloudSelectedKey: DEMO_CLOUD_SELECTED_KEY,
    routingLogs: DEMO_ROUTING_LOGS,
  }),
  clear: () => set({
    active: false,
    stats: null,
    globalStats: null,
    sessionSavedPoints: null,
    globalSavedPoints: null,
    status: null,
    edgeModels: [],
    edgeSelectedKey: null,
    cloudModels: [],
    cloudSelectedKey: null,
    routingLogs: [],
  }),
}))
