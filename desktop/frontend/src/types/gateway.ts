export type RouteMode = 'auto' | 'edge' | 'cloud' | 'cascade'
export type RoutingMode = 'single' | 'cascade' | 'split'
export type Profile = 'economy' | 'balanced' | 'premium' | 'privacy'
export type StatsScope = 'session' | 'global'
export type ThemePref = 'light' | 'dark' | 'system'

export interface GatewayStatus {
  service: string
  status: 'running' | 'stopped'
  version: string
  listen?: string
  listen_lan?: boolean
  lan_base_url?: string | null
  pid?: number
  uptime_secs: number
  edge_configured: boolean
  cloud_configured: boolean
  default_profile?: Profile
  route?: RouteMode
  data_dir?: string
}

export interface GatewayConfigView {
  route: RouteMode
  routing_mode: RoutingMode
  default_profile: Profile
  ctx_edge_max_tokens?: number
  experience_enabled?: boolean
  experience_learning_rate?: number
  experience_target_fallback?: number
  cloud_cache_decay_half_life_secs?: number
  cloud_cache_boost_max?: number
  request_route_cache_enabled?: boolean
  request_route_cache_retention_days?: number
  request_route_cache_cleanup_interval_secs?: number
  /** @deprecated use cloud_cache_decay_half_life_secs */
  cloud_sticky_ttl_secs?: number
  work_verify_sample_rate?: number
  adaptive_routing_enabled?: boolean
  adaptive_min_verified_samples?: number
  adaptive_verify_rate_floor?: number
  adaptive_verify_rate_ceiling?: number
  adaptive_max_theta_shift?: number
  classifier_enabled?: boolean
  classifier_min_samples?: number
  classifier_prior_alpha?: number
  classifier_decay_half_life_hours?: number
  classifier_prior_from_heuristic?: boolean
  listen_port?: number
  listen_lan?: boolean
  auth_enabled?: boolean
  api_key_set?: boolean
  api_key_preview?: string | null
}

export interface UpstreamEndpointView {
  configured: boolean
  base_url: string
  model?: string | null
  api_key_set?: boolean
  token_budget?: number | null
  token_quota_enabled?: boolean
}

export interface UpstreamSetupView {
  gateway: GatewayConfigView
  edge?: UpstreamEndpointView | null
  cloud?: UpstreamEndpointView | null
  agent_id?: string | null
}

export interface UpstreamSetupUpdate {
  agent_id?: string | null
  gateway?: Partial<GatewayConfigView> & { api_key?: string }
  cloud?: {
    base_url?: string
    model?: string
    api_key?: string
    token_budget?: number | null
    clear?: boolean
  }
  edge?: {
    base_url?: string
    model?: string | null
    api_key?: string
    clear?: boolean
  }
}

export interface RouteCounts {
  edge: number
  cloud: number
  cascade: number
  edge_pct: number
  cloud_pct: number
  cascade_pct: number
}

export interface ErrorCounts {
  total?: number
  unauthorized?: number
  unavailable?: number
  upstream?: number
  bad_request?: number
}

export interface LatencyStats {
  avg_request_ms?: number
  avg_ttft_ms?: number
  avg_tps?: number
  edge_tps?: number
  cloud_tps?: number
  p95_ms?: number
  p99_ms?: number
}

export interface ModelTokenStats {
  tier: string
  model: string
  input: number
  output: number
  cached: number
  last_used_at_unix?: number | null
}

export interface ModelTimelinePoint {
  bucket_ts: number
  input: number
  output: number
  cached: number
}

export interface ModelTimelineResponse {
  scope: string
  tier: string
  model: string
  range: string
  granularity: string
  points: ModelTimelinePoint[]
}

export interface ModelTimelineSeries {
  tier: string
  model: string
  points: ModelTimelinePoint[]
}

export interface AllModelsTimelineResponse {
  scope: string
  range: string
  granularity: string
  models: ModelTimelineSeries[]
}

export interface StatsTimelinePoint {
  bucket_ts: number
  edge_in: number
  edge_out: number
  cloud_in: number
  cloud_out: number
  requests_total: number
}

export interface StatsTimelineResponse {
  scope: string
  range: string
  granularity: string
  points: StatsTimelinePoint[]
}

export interface AuthKeyTokenStats {
  input: number
  output: number
  total: number
}

export interface AuthKeyLatencyStats {
  avg_request_ms: number
  avg_tps: number
  edge_tps: number
  cloud_tps: number
}

export interface AuthKeyStatsSnapshot {
  id: string
  name: string
  key_preview: string
  deleted: boolean
  last_used_at_unix?: number | null
  requests_total: number
  tokens: AuthKeyTokenStats
  latency: AuthKeyLatencyStats
  routing: RouteCounts
}

export interface StatsSnapshot {
  scope: StatsScope | string
  requests_total: number
  requests_cancelled?: number
  requests_per_minute?: number
  routing: RouteCounts
  cascade?: { edge_ok?: number; fallback_to_cloud?: number }
  tokens?: Record<string, number>
  token_breakdown?: Record<string, unknown>
  latency?: LatencyStats
  errors?: ErrorCounts
  step_kinds?: Record<string, number>
  effective_routing?: Record<string, unknown> | null
  classifier?: Record<string, unknown> | null
  agent_budgets?: Array<{ agent_id: string; budget_limit: number | null; tokens_used: number }>
  auth_key_stats?: AuthKeyStatsSnapshot[] | null
  model_stats?: ModelTokenStats[]
}

export interface SetupPostResponse {
  ok: boolean
  message: string
  upstream: UpstreamSetupView
}

export interface LogsResponse {
  offset: number
  next_offset: number
  reset: boolean
  lines: Array<{ level: string; text?: string; msg?: string }>
}

export interface RoutingLogApiEntry {
  served_model?: string | null
  id: number
  timestamp: string
  route: string
  served_route?: string | null
  step_kind: string
  model: string
  user_preview: string
  difficulty?: number
  reason_codes: string[]
}

export interface RoutingLogsResponse {
  entries: RoutingLogApiEntry[]
  has_older: boolean
}

export interface LogLine {
  level: string
  msg: string
}

export function emptySetup(): UpstreamSetupView {
  return {
    gateway: { route: 'auto', routing_mode: 'cascade', default_profile: 'economy', ctx_edge_max_tokens: 200000 },
    cloud: { configured: false, base_url: '' },
    edge: { configured: false, base_url: '' },
  }
}

export function emptyStatus(): GatewayStatus {
  return {
    service: 'token-router',
    status: 'stopped',
    version: '—',
    uptime_secs: 0,
    edge_configured: false,
    cloud_configured: false,
  }
}

export function emptyStats(scope: StatsScope = 'session'): StatsSnapshot {
  return {
    scope,
    requests_total: 0,
    routing: { edge: 0, cloud: 0, cascade: 0, edge_pct: 0, cloud_pct: 0, cascade_pct: 0 },
    cascade: {},
    tokens: {},
    token_breakdown: {},
    latency: {},
    step_kinds: {},
    agent_budgets: [],
  }
}
