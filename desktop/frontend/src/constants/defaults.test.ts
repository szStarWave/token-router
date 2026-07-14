import assert from 'node:assert/strict'
import test from 'node:test'
import { pickStrategyGatewayPatch, STRATEGY_FIELD_KEYS } from './defaults.js'

test('pickStrategyGatewayPatch keeps all strategy fields', () => {
  const src = {
    route: 'edge' as const,
    routing_mode: 'cascade' as const,
    default_profile: 'premium' as const,
    ctx_edge_max_tokens: 300_000,
    experience_enabled: true,
    work_verify_sample_rate: 0.2,
    cloud_cache_decay_half_life_secs: 1200,
    cloud_cache_boost_max: 0.25,
    experience_learning_rate: 0.1,
    experience_target_fallback: 0.2,
    adaptive_routing_enabled: true,
    adaptive_min_verified_samples: 30,
    adaptive_verify_rate_floor: 0.1,
    adaptive_verify_rate_ceiling: 0.5,
    adaptive_max_theta_shift: 0.08,
    classifier_enabled: true,
    classifier_prior_from_heuristic: false,
    classifier_min_samples: 200,
    classifier_prior_alpha: 2.0,
    classifier_decay_half_life_hours: 336,
  }
  const result = pickStrategyGatewayPatch(src)
  for (const key of STRATEGY_FIELD_KEYS) {
    assert.equal(key in result, true, `expected ${key} in patch`)
    assert.equal(result[key], src[key], `expected ${key} to keep its value`)
  }
})

test('pickStrategyGatewayPatch strips non-strategy fields', () => {
  const src = {
    route: 'auto' as const,
    routing_mode: 'cascade' as const,
    default_profile: 'balanced' as const,
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
    // Non-strategy fields that must be stripped:
    listen_port: 11088,
    listen_lan: true,
    auth_enabled: true,
    api_key_set: true,
    request_route_cache_enabled: false,
    request_route_cache_retention_days: 30,
    request_route_cache_cleanup_interval_secs: 3600,
    ['cloud_sticky_ttl_secs' as string]: 999,
  }
  const result = pickStrategyGatewayPatch(src)
  // Strategy fields should still be present
  assert.equal(result.route, 'auto')
  assert.equal(result.routing_mode, 'cascade')
  // Non-strategy fields must be absent
  assert.equal('listen_port' in result, false)
  assert.equal('listen_lan' in result, false)
  assert.equal('auth_enabled' in result, false)
  assert.equal('api_key_set' in result, false)
  assert.equal('request_route_cache_enabled' in result, false)
  assert.equal('request_route_cache_retention_days' in result, false)
  assert.equal('request_route_cache_cleanup_interval_secs' in result, false)
  assert.equal('cloud_sticky_ttl_secs' in result, false)
})

test('pickStrategyGatewayPatch returns empty object for empty input', () => {
  const result = pickStrategyGatewayPatch({})
  assert.equal(Object.keys(result).length, 0)
})

test('pickStrategyGatewayPatch includes only fields present in src', () => {
  const result = pickStrategyGatewayPatch({ route: 'cloud' })
  assert.equal(Object.keys(result).length, 1)
  assert.equal(result.route, 'cloud')
})
