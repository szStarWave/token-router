use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::config::{UpstreamSetupUpdate, is_setup_validation_error};
use crate::gateway::api::routes::AppState;

pub async fn setup_page() -> Html<&'static str> {
    Html(SETUP_HTML)
}

pub async fn setup_get(
    State(state): State<AppState>,
    Query(params): Query<AgentQueryParams>,
) -> impl IntoResponse {
    match state.config_mgr.setup_view_for_agent(params.agent_id.as_deref()) {
        Ok(view) => Json(view).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn setup_init(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match state.config_mgr.write_default_setup() {
        Ok(view) => Json(SetupResponse {
            ok: true,
            message: "default upstream setup applied (cloud model=auto, edge empty)",
            upstream: view,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn setup_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<UpstreamSetupUpdate>,
) -> Response {
    if let Some(resp) = require_admin(&state, &headers) {
        return resp;
    }
    match state.config_mgr.apply_setup_with_config(&patch) {
        Ok((view, config)) => {
            state.apply_runtime_config(&config);
            Json(SetupResponse {
                ok: true,
                message: "setup updated",
                upstream: view,
            })
            .into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            let status = if is_setup_validation_error(&msg) {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({"error": msg}))).into_response()
        }
    }
}

#[derive(Serialize)]
struct SetupResponse {
    ok: bool,
    message: &'static str,
    upstream: crate::config::UpstreamSetupView,
}

#[derive(Deserialize, Default)]
pub struct AgentQueryParams {
    pub agent_id: Option<String>,
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let config = state.config_mgr.get();
    let Some(expected) = config.admin_token.as_ref() else {
        return None;
    };
    let provided = headers
        .get("x-token-router-admin-token")
        .and_then(|v| v.to_str().ok());
    if provided == Some(expected.as_str()) {
        return None;
    }
    Some(
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid admin token"})),
        )
            .into_response(),
    )
}

const SETUP_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Token Router — 配置</title>
  <style>
    :root { color-scheme: light dark; font-family: system-ui, sans-serif; }
    body { max-width: 720px; margin: 2rem auto; padding: 0 1rem; line-height: 1.5; }
    h1 { font-size: 1.35rem; }
    fieldset { border: 1px solid #8884; border-radius: 8px; margin: 1rem 0; padding: 1rem; }
    legend { padding: 0 0.4rem; font-weight: 600; }
    label { display: block; margin: 0.6rem 0 0.2rem; font-size: 0.9rem; }
    input { width: 100%; box-sizing: border-box; padding: 0.45rem 0.55rem; border-radius: 6px; border: 1px solid #8886; }
    .row { display: flex; gap: 0.75rem; flex-wrap: wrap; }
    button { margin-top: 1rem; margin-right: 0.5rem; padding: 0.5rem 1rem; border-radius: 6px; border: 1px solid #8886; cursor: pointer; }
    #status { margin-top: 1rem; white-space: pre-wrap; font-size: 0.9rem; }
    .hint { color: #888; font-size: 0.85rem; }
  </style>
</head>
<body>
  <h1>Token Router — 配置</h1>
  <p class="hint">路由、经验学习、上游 URL 等保存后立即生效（<code>session_persist_enabled</code> 需重启）。云端 model 默认 <code>auto</code>。</p>
  <label for="agent_id">Agent ID（留空=全局默认；指定后下方配置仅对该 agent 生效）</label>
  <input id="agent_id" placeholder="" />
  <label for="admin_token">Admin Token（若 config 中配置了 admin_token）</label>
  <input id="admin_token" type="password" placeholder="X-Token-Router-Admin-Token" autocomplete="off" />

  <fieldset>
    <legend>路由 gateway</legend>
    <label for="route">route</label>
    <select id="route">
      <option value="auto">auto</option>
      <option value="edge">edge</option>
      <option value="cloud">cloud</option>
      <option value="cascade">cascade</option>
    </select>
    <label for="routing_mode">routing_mode（route=auto）</label>
    <select id="routing_mode">
      <option value="single">single</option>
      <option value="cascade">cascade</option>
      <option value="split">split</option>
    </select>
    <label for="default_profile">default_profile</label>
    <select id="default_profile">
      <option value="economy">economy</option>
      <option value="balanced">balanced</option>
      <option value="premium">premium</option>
      <option value="privacy">privacy</option>
    </select>
    <label for="ctx_edge_max">ctx_edge_max_tokens（4096–2000000）</label>
    <input id="ctx_edge_max" type="number" min="4096" max="2000000" step="1024" placeholder="100000" />
    <p class="hint">超过约 80% 触发 GATE_CTX_OVERFLOW 升云。</p>
  </fieldset>

  <fieldset>
    <legend>经验学习</legend>
    <label><input id="experience_enabled" type="checkbox" /> experience_enabled</label>
    <label for="experience_learning_rate">experience_learning_rate (0–1)</label>
    <input id="experience_learning_rate" type="number" min="0" max="1" step="0.01" />
    <label for="experience_max_bias">experience_max_bias (0–1)</label>
    <input id="experience_max_bias" type="number" min="0" max="1" step="0.01" />
    <label for="experience_target_fallback">experience_target_fallback (0–1)</label>
    <input id="experience_target_fallback" type="number" min="0" max="1" step="0.01" />
    <label for="cloud_sticky_ttl_secs">cloud_sticky_ttl_secs</label>
    <input id="cloud_sticky_ttl_secs" type="number" min="0" max="604800" step="60" />
    <label><input id="session_persist_enabled" type="checkbox" /> session_persist_enabled（重启生效）</label>
    <label for="work_verify_sample_rate">work_verify_sample_rate (0–1)</label>
    <input id="work_verify_sample_rate" type="number" min="0" max="1" step="0.05" />
  </fieldset>

  <fieldset>
    <legend>自适应路由（内存）</legend>
    <label><input id="adaptive_routing_enabled" type="checkbox" /> adaptive_routing_enabled</label>
    <label for="adaptive_min_verified_samples">adaptive_min_verified_samples</label>
    <input id="adaptive_min_verified_samples" type="number" min="1" max="1000000" step="1" />
    <label for="adaptive_verify_rate_floor">adaptive_verify_rate_floor</label>
    <input id="adaptive_verify_rate_floor" type="number" min="0" max="1" step="0.01" />
    <label for="adaptive_verify_rate_ceiling">adaptive_verify_rate_ceiling</label>
    <input id="adaptive_verify_rate_ceiling" type="number" min="0" max="1" step="0.01" />
    <label for="adaptive_max_theta_shift">adaptive_max_theta_shift (0–0.5)</label>
    <input id="adaptive_max_theta_shift" type="number" min="0" max="0.5" step="0.01" />
  </fieldset>

  <fieldset>
    <legend>云端 Cloud</legend>
    <label for="cloud_url">Base URL（OpenAI 兼容，含 /v1）</label>
    <input id="cloud_url" placeholder="https://api.deepseek.com/v1" />
    <label for="cloud_model">Model</label>
    <input id="cloud_model" placeholder="auto" />
    <label for="cloud_key">API Key</label>
    <input id="cloud_key" type="password" placeholder="留空则不修改已保存的 key" autocomplete="off" />
    <label for="cloud_token_budget">Token 预算（5 小时窗口，超预算 Cloud 降级为 Cascade；0=不限）</label>
    <input id="cloud_token_budget" type="number" min="0" step="10000" placeholder="如 500000" />
  </fieldset>

  <fieldset>
    <legend>端侧 Edge</legend>
    <label for="edge_url">Base URL</label>
    <input id="edge_url" placeholder="http://127.0.0.1:11434/v1" />
    <label for="edge_model">Model（可选，空=auto）</label>
    <input id="edge_model" placeholder="" />
    <label for="edge_key">API Key</label>
    <input id="edge_key" type="password" placeholder="留空则不修改" autocomplete="off" />
    <label><input id="edge_clear" type="checkbox" /> 清除端侧配置</label>
  </fieldset>

  <div class="row">
    <button type="button" id="load">加载当前配置</button>
    <button type="button" id="defaults">恢复默认（cloud=auto，edge 空）</button>
    <button type="button" id="save">保存</button>
  </div>
  <div id="status"></div>

  <script>
    const status = document.getElementById('status');
    function headers() {
      const h = { 'Content-Type': 'application/json' };
      const t = document.getElementById('admin_token').value.trim();
      if (t) h['X-Token-Router-Admin-Token'] = t;
      return h;
    }
    function fill(view) {
      const g = view.gateway || {};
      const cloud = view.cloud || {};
      const edge = view.edge || {};
      document.getElementById('route').value = g.route || 'auto';
      document.getElementById('routing_mode').value = g.routing_mode || 'cascade';
      document.getElementById('default_profile').value = g.default_profile || 'balanced';
      document.getElementById('ctx_edge_max').value = g.ctx_edge_max_tokens || '';
      document.getElementById('experience_enabled').checked = !!g.experience_enabled;
      document.getElementById('experience_learning_rate').value = g.experience_learning_rate ?? '';
      document.getElementById('experience_max_bias').value = g.experience_max_bias ?? '';
      document.getElementById('experience_target_fallback').value = g.experience_target_fallback ?? '';
      document.getElementById('cloud_sticky_ttl_secs').value = g.cloud_sticky_ttl_secs ?? '';
      document.getElementById('session_persist_enabled').checked = !!g.session_persist_enabled;
      document.getElementById('work_verify_sample_rate').value = g.work_verify_sample_rate ?? '';
      document.getElementById('adaptive_routing_enabled').checked = !!g.adaptive_routing_enabled;
      document.getElementById('adaptive_min_verified_samples').value = g.adaptive_min_verified_samples ?? '';
      document.getElementById('adaptive_verify_rate_floor').value = g.adaptive_verify_rate_floor ?? '';
      document.getElementById('adaptive_verify_rate_ceiling').value = g.adaptive_verify_rate_ceiling ?? '';
      document.getElementById('adaptive_max_theta_shift').value = g.adaptive_max_theta_shift ?? '';
      document.getElementById('agent_id').value = view.agent_id || '';
      document.getElementById('cloud_token_budget').value = cloud.token_budget ?? '';
      document.getElementById('cloud_url').value = cloud.base_url || '';
      document.getElementById('cloud_model').value = cloud.model || 'auto';
      document.getElementById('edge_url').value = edge.base_url || '';
      document.getElementById('edge_model').value = edge.model || '';
      document.getElementById('edge_clear').checked = false;
      document.getElementById('cloud_key').value = '';
      document.getElementById('edge_key').value = '';
    }
    async function load() {
      status.textContent = 'Loading…';
      try {
        const agentId = document.getElementById('agent_id').value.trim();
        const url = agentId ? '/v1/admin/setup?agent_id=' + encodeURIComponent(agentId) : '/v1/admin/setup';
        const r = await fetch(url);
        const j = await r.json();
        if (!r.ok) throw new Error(j.error || r.statusText);
        fill(j);
        status.textContent = 'Loaded.';
      } catch (e) { status.textContent = 'Error: ' + e.message; }
    }
    async function defaults() {
      status.textContent = 'Applying defaults…';
      try {
        const r = await fetch('/v1/admin/setup/init', { method: 'POST', headers: headers() });
        const j = await r.json();
        if (!r.ok) throw new Error(j.error || r.statusText);
        fill(j.upstream);
        status.textContent = j.message || 'OK';
      } catch (e) { status.textContent = 'Error: ' + e.message; }
    }
    async function save() {
      status.textContent = 'Saving…';
      const num = (id) => {
        const v = document.getElementById(id).value.trim();
        return v === '' ? undefined : Number(v);
      };
      const agentIdVal = document.getElementById('agent_id').value.trim();
      const body = {
        agent_id: agentIdVal || null,
        gateway: {
          route: document.getElementById('route').value,
          routing_mode: document.getElementById('routing_mode').value,
          default_profile: document.getElementById('default_profile').value,
          ctx_edge_max_tokens: num('ctx_edge_max'),
          experience_enabled: document.getElementById('experience_enabled').checked,
          experience_learning_rate: num('experience_learning_rate'),
          experience_max_bias: num('experience_max_bias'),
          experience_target_fallback: num('experience_target_fallback'),
          cloud_sticky_ttl_secs: num('cloud_sticky_ttl_secs'),
          session_persist_enabled: document.getElementById('session_persist_enabled').checked,
          work_verify_sample_rate: num('work_verify_sample_rate'),
          adaptive_routing_enabled: document.getElementById('adaptive_routing_enabled').checked,
          adaptive_min_verified_samples: num('adaptive_min_verified_samples'),
          adaptive_verify_rate_floor: num('adaptive_verify_rate_floor'),
          adaptive_verify_rate_ceiling: num('adaptive_verify_rate_ceiling'),
          adaptive_max_theta_shift: num('adaptive_max_theta_shift'),
        },
        cloud: {
          base_url: document.getElementById('cloud_url').value,
          model: document.getElementById('cloud_model').value || 'auto',
          token_budget: num('cloud_token_budget'),
        },
        edge: {
          clear: document.getElementById('edge_clear').checked,
          base_url: document.getElementById('edge_url').value,
          model: document.getElementById('edge_model').value || null,
        }
      };
      const ck = document.getElementById('cloud_key').value;
      const ek = document.getElementById('edge_key').value;
      if (ck) body.cloud.api_key = ck;
      if (ek) body.edge.api_key = ek;
      try {
        const r = await fetch('/v1/admin/setup', { method: 'POST', headers: headers(), body: JSON.stringify(body) });
        const j = await r.json();
        if (!r.ok) throw new Error(j.error || r.statusText);
        fill(j.upstream);
        status.textContent = j.message || 'Saved.';
      } catch (e) { status.textContent = 'Error: ' + e.message; }
    }
    document.getElementById('load').onclick = load;
    document.getElementById('defaults').onclick = defaults;
    document.getElementById('save').onclick = save;
    load();
  </script>
</body>
</html>
"#;
