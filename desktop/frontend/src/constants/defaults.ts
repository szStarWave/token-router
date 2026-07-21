import type { GatewayConfigView } from '../types/gateway'

/**
 * Whitelist of field keys that belong to the strategy (routing) page.
 * Any field NOT in this list (listen_port, auth_enabled, session_persist, etc.)
 * is intentionally excluded when saving or resetting on the strategy page.
 */
export const STRATEGY_FIELD_KEYS: ReadonlyArray<keyof GatewayConfigView> = [
  'route',
  'routing_mode',
  'default_profile',
  'ctx_edge_max_tokens',
  'experience_enabled',
  'work_verify_sample_rate',
  'cloud_cache_decay_half_life_secs',
  'cloud_cache_boost_max',
  'experience_learning_rate',
  'experience_target_fallback',
  'adaptive_routing_enabled',
  'adaptive_min_verified_samples',
  'adaptive_verify_rate_floor',
  'adaptive_verify_rate_ceiling',
  'adaptive_max_theta_shift',
  'classifier_enabled',
  'classifier_prior_from_heuristic',
  'classifier_min_samples',
  'classifier_prior_alpha',
  'classifier_decay_half_life_hours',
]

/** Keep only strategy-page fields from a partial `GatewayConfigView`. */
export function pickStrategyGatewayPatch(
  src: Partial<GatewayConfigView>,
): Partial<GatewayConfigView> {
  const patch: Record<string, unknown> = {}
  for (const key of STRATEGY_FIELD_KEYS) {
    if (key in src) {
      patch[key] = src[key]
    }
  }
  return patch as Partial<GatewayConfigView>
}

/** Strategy page defaults — mirrors `GatewaySection::default()` in `src/config/file.rs`. */
export const GATEWAY_ROUTING_DEFAULTS: Partial<GatewayConfigView> = {
  route: 'auto',
  routing_mode: 'cascade',
  default_profile: 'balanced',
  ctx_edge_max_tokens: 200_000,
  experience_enabled: true,
  work_verify_sample_rate: 0.1,
  cloud_cache_decay_half_life_secs: 600,
  cloud_cache_boost_max: 0.18,
  experience_learning_rate: 0.08,
  experience_target_fallback: 0.15,
  adaptive_routing_enabled: true,
  adaptive_min_verified_samples: 20,
  adaptive_verify_rate_floor: 0.05,
  adaptive_verify_rate_ceiling: 0.45,
  adaptive_max_theta_shift: 0.05,
  classifier_enabled: true,
  classifier_prior_from_heuristic: true,
  classifier_min_samples: 100,
  classifier_prior_alpha: 1.0,
  classifier_decay_half_life_hours: 168,
}

export const DEFAULT_GATEWAY_PORT = 11_088
export const DEFAULT_GATEWAY_BASE = 'http://127.0.0.1:11088'
export const DEFAULT_CLOUD_TOKEN_BUDGET = 1_000_000
export const CLOUD_BUDGET_MIN = 10_000
export const CLOUD_BUDGET_MAX = 1_000_000_000_000
export const CLOUD_BUDGET_STEP = 10_000
export const CLOUD_BUDGET_SLIDER_STEPS = 1_000
export const LOG_MAX_LINES = 2_000
/** Only load older log bytes when the scroll thumb is essentially at the top. */
export const LOG_LOAD_OLDER_THRESHOLD = 8
export const STORAGE_THEME = 'tr-theme'
export const STORAGE_LOCALE = 'tr-locale'
