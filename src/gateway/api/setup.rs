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

pub fn require_admin(state: &AppState, headers: &HeaderMap) -> Option<Response> {
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
    body { max-width: 800px; margin: 2rem auto; padding: 0 1rem; line-height: 1.5; }
    h1 { font-size: 1.35rem; }
    h2 { font-size: 1.05rem; margin: 1.5rem 0 0.4rem; border-bottom: 1px solid #8884; padding-bottom: 0.25rem; }
    fieldset { border: 1px solid #8884; border-radius: 8px; margin: 1rem 0; padding: 1rem; }
    legend { padding: 0 0.4rem; font-weight: 600; }
    label { display: block; margin: 0.6rem 0 0.2rem; font-size: 0.9rem; }
    input, select { width: 100%; box-sizing: border-box; padding: 0.45rem 0.55rem; border-radius: 6px; border: 1px solid #8886; }
    input[type=checkbox] { width: auto; }
    .row { display: flex; gap: 0.75rem; flex-wrap: wrap; }
    button { margin-top: 1rem; margin-right: 0.5rem; padding: 0.5rem 1rem; border-radius: 6px; border: 1px solid #8886; cursor: pointer; }
    #status { margin-top: 1rem; white-space: pre-wrap; font-size: 0.9rem; }
    .hint { color: #888; font-size: 0.85rem; }
  </style>
</head>
<body>
  <h1>Token Router — 配置</h1>
  <p class="hint">可配置 LLM / 文生图 / 文生视频上游。保存后立即生效（<code>session_persist_enabled</code> 需重启）。Image / Video 始终为全局配置（不受 Agent ID 影响）。</p>
  <label for="agent_id">Agent ID（留空=全局默认；仅影响 LLM 上游与路由）</label>
  <input id="agent_id" placeholder="" />
  <label for="admin_token">Admin Token（若 config 中配置了 admin_token）</label>
  <input id="admin_token" type="password" placeholder="X-Token-Router-Admin-Token" autocomplete="off" />

  <h2>LLM 路由</h2>
  <fieldset>
    <legend>gateway.route</legend>
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

  <h2>LLM 上游</h2>
  <fieldset>
    <legend>Cloud（chat）</legend>
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
    <legend>Edge（chat）</legend>
    <label for="edge_url">Base URL</label>
    <input id="edge_url" placeholder="http://127.0.0.1:11434/v1" />
    <label for="edge_model">Model（可选，空=auto）</label>
    <input id="edge_model" placeholder="" />
    <label for="edge_key">API Key</label>
    <input id="edge_key" type="password" placeholder="留空则不修改" autocomplete="off" />
    <label><input id="edge_clear" type="checkbox" /> 清除端侧配置</label>
  </fieldset>

  <h2>文生图 / 图生图</h2>
  <fieldset>
    <legend>image_route</legend>
    <label for="image_route">image_route（与 chat route 独立）</label>
    <select id="image_route">
      <option value="auto">auto</option>
      <option value="edge">edge</option>
      <option value="cloud">cloud</option>
    </select>
    <p class="hint">auto 优先 edge；端侧忙且已配 cloud 时直接升云。</p>
  </fieldset>
  <fieldset>
    <legend>Image Cloud</legend>
    <label for="image_cloud_provider">provider</label>
    <select id="image_cloud_provider">
      <option value="openai">openai</option>
      <option value="dashscope">dashscope</option>
      <option value="seedream">seedream</option>
      <option value="comfyui">comfyui</option>
    </select>
    <label for="image_cloud_url">Base URL</label>
    <input id="image_cloud_url" placeholder="https://api.openai.com/v1" />
    <label for="image_cloud_model">Model</label>
    <input id="image_cloud_model" placeholder="gpt-image-1" />
    <label for="image_cloud_upstream_model">upstream_model（可选）</label>
    <input id="image_cloud_upstream_model" placeholder="" />
    <label for="image_cloud_key">API Key</label>
    <input id="image_cloud_key" type="password" placeholder="留空则不修改" autocomplete="off" />
    <label><input id="image_cloud_clear" type="checkbox" /> 清除 Image Cloud</label>
  </fieldset>
  <fieldset>
    <legend>Image Edge</legend>
    <label for="image_edge_provider">provider</label>
    <select id="image_edge_provider">
      <option value="comfyui">comfyui</option>
      <option value="openai">openai</option>
      <option value="dashscope">dashscope</option>
      <option value="seedream">seedream</option>
    </select>
    <label for="image_edge_url">Base URL</label>
    <input id="image_edge_url" placeholder="http://127.0.0.1:8188" />
    <label for="image_edge_model">Model / ckpt</label>
    <input id="image_edge_model" placeholder="v1-5-pruned-emaonly.safetensors" />
    <label for="image_edge_upstream_model">upstream_model（可选）</label>
    <input id="image_edge_upstream_model" placeholder="" />
    <label for="image_edge_key">API Key</label>
    <input id="image_edge_key" type="password" placeholder="留空则不修改" autocomplete="off" />
    <label for="image_edge_workflow">workflow_file（T2I，可选）</label>
    <input id="image_edge_workflow" placeholder="comfy-t2i-workflow.json" />
    <label for="image_edge_workflow_i2i">workflow_file_i2i（可选）</label>
    <input id="image_edge_workflow_i2i" placeholder="comfy-i2i-workflow.json" />
    <label><input id="image_edge_clear" type="checkbox" /> 清除 Image Edge</label>
  </fieldset>

  <h2>文生视频 / 图生视频</h2>
  <fieldset>
    <legend>video_route</legend>
    <label for="video_route">video_route（与 chat/image 独立）</label>
    <select id="video_route">
      <option value="auto">auto</option>
      <option value="edge">edge</option>
      <option value="cloud">cloud</option>
    </select>
    <p class="hint">auto 优先 edge；端侧忙且已配 cloud 时直接升云。</p>
  </fieldset>
  <fieldset>
    <legend>Video Cloud</legend>
    <label for="video_cloud_provider">provider</label>
    <select id="video_cloud_provider">
      <option value="openai">openai</option>
      <option value="dashscope">dashscope</option>
      <option value="seedance">seedance</option>
      <option value="minimax">minimax</option>
      <option value="comfyui">comfyui</option>
    </select>
    <label for="video_cloud_url">Base URL</label>
    <input id="video_cloud_url" placeholder="https://api.openai.com/v1" />
    <label for="video_cloud_model">Model</label>
    <input id="video_cloud_model" placeholder="sora-2" />
    <label for="video_cloud_upstream_model">upstream_model（可选）</label>
    <input id="video_cloud_upstream_model" placeholder="" />
    <label for="video_cloud_key">API Key</label>
    <input id="video_cloud_key" type="password" placeholder="留空则不修改" autocomplete="off" />
    <label><input id="video_cloud_clear" type="checkbox" /> 清除 Video Cloud</label>
  </fieldset>
  <fieldset>
    <legend>Video Edge</legend>
    <label for="video_edge_provider">provider</label>
    <select id="video_edge_provider">
      <option value="comfyui">comfyui</option>
      <option value="openai">openai</option>
      <option value="dashscope">dashscope</option>
      <option value="seedance">seedance</option>
      <option value="minimax">minimax</option>
    </select>
    <label for="video_edge_url">Base URL</label>
    <input id="video_edge_url" placeholder="http://127.0.0.1:8188" />
    <label for="video_edge_model">Model / ckpt</label>
    <input id="video_edge_model" placeholder="your-video-ckpt.safetensors" />
    <label for="video_edge_upstream_model">upstream_model（可选）</label>
    <input id="video_edge_upstream_model" placeholder="" />
    <label for="video_edge_key">API Key</label>
    <input id="video_edge_key" type="password" placeholder="留空则不修改" autocomplete="off" />
    <label for="video_edge_workflow">workflow_file（T2V，可选）</label>
    <input id="video_edge_workflow" placeholder="comfy-t2v-workflow.json" />
    <label for="video_edge_workflow_i2v">workflow_file_i2v（可选）</label>
    <input id="video_edge_workflow_i2v" placeholder="comfy-i2v-workflow.json" />
    <label><input id="video_edge_clear" type="checkbox" /> 清除 Video Edge</label>
  </fieldset>

  <div class="row">
    <button type="button" id="load">加载当前配置</button>
    <button type="button" id="defaults">恢复默认（LLM cloud=auto，edge 空）</button>
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
    function val(id, fallback) {
      const el = document.getElementById(id);
      if (!el) return fallback;
      return el.value;
    }
    function setVal(id, v) {
      const el = document.getElementById(id);
      if (el) el.value = v == null ? '' : v;
    }
    function setCheck(id, v) {
      const el = document.getElementById(id);
      if (el) el.checked = !!v;
    }
    function fillMedia(prefix, ep) {
      const e = ep || {};
      setVal(prefix + '_provider', e.provider || (prefix.indexOf('edge') >= 0 ? 'comfyui' : 'openai'));
      setVal(prefix + '_url', e.base_url || '');
      setVal(prefix + '_model', e.model || '');
      setVal(prefix + '_upstream_model', e.upstream_model || '');
      setVal(prefix + '_key', '');
      setCheck(prefix + '_clear', false);
      if (prefix === 'image_edge') {
        setVal('image_edge_workflow', e.workflow_file || '');
        setVal('image_edge_workflow_i2i', e.workflow_file_i2i || '');
      }
      if (prefix === 'video_edge') {
        setVal('video_edge_workflow', e.workflow_file || '');
        setVal('video_edge_workflow_i2v', e.workflow_file_i2v || '');
      }
    }
    function fill(view) {
      const g = view.gateway || {};
      const cloud = view.cloud || {};
      const edge = view.edge || {};
      setVal('route', g.route || 'auto');
      setVal('routing_mode', g.routing_mode || 'cascade');
      setVal('default_profile', g.default_profile || 'balanced');
      setVal('ctx_edge_max', g.ctx_edge_max_tokens || '');
      setCheck('experience_enabled', g.experience_enabled);
      setVal('experience_learning_rate', g.experience_learning_rate ?? '');
      setVal('experience_max_bias', g.experience_max_bias ?? '');
      setVal('experience_target_fallback', g.experience_target_fallback ?? '');
      setVal('cloud_sticky_ttl_secs', g.cloud_sticky_ttl_secs ?? '');
      setCheck('session_persist_enabled', g.session_persist_enabled);
      setVal('work_verify_sample_rate', g.work_verify_sample_rate ?? '');
      setCheck('adaptive_routing_enabled', g.adaptive_routing_enabled);
      setVal('adaptive_min_verified_samples', g.adaptive_min_verified_samples ?? '');
      setVal('adaptive_verify_rate_floor', g.adaptive_verify_rate_floor ?? '');
      setVal('adaptive_verify_rate_ceiling', g.adaptive_verify_rate_ceiling ?? '');
      setVal('adaptive_max_theta_shift', g.adaptive_max_theta_shift ?? '');
      setVal('image_route', g.image_route || 'auto');
      setVal('video_route', g.video_route || 'auto');
      setVal('agent_id', view.agent_id || '');
      setVal('cloud_token_budget', cloud.token_budget ?? '');
      setVal('cloud_url', cloud.base_url || '');
      setVal('cloud_model', cloud.model || 'auto');
      setVal('edge_url', edge.base_url || '');
      setVal('edge_model', edge.model || '');
      setCheck('edge_clear', false);
      setVal('cloud_key', '');
      setVal('edge_key', '');
      fillMedia('image_cloud', view.image_cloud);
      fillMedia('image_edge', view.image_edge);
      fillMedia('video_cloud', view.video_cloud);
      fillMedia('video_edge', view.video_edge);
    }
    function mediaPatch(prefix, opts) {
      const clear = document.getElementById(prefix + '_clear').checked;
      const url = val(prefix + '_url').trim();
      const key = val(prefix + '_key');
      if (!clear && !url && !key) {
        // Skip untouched empty media slots so save does not churn config.
        return undefined;
      }
      const patch = { clear: clear };
      if (clear) return patch;
      patch.provider = val(prefix + '_provider');
      patch.base_url = url;
      const model = val(prefix + '_model').trim();
      patch.model = model === '' ? null : model;
      const um = val(prefix + '_upstream_model').trim();
      patch.upstream_model = um;
      if (key) patch.api_key = key;
      if (opts && opts.workflow) {
        const wf = val(opts.workflow).trim();
        patch.workflow_file = wf === '' ? null : wf;
      }
      if (opts && opts.workflowAlt) {
        const wf2 = val(opts.workflowAlt).trim();
        patch[opts.workflowAltKey] = wf2 === '' ? null : wf2;
      }
      return patch;
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
          image_route: document.getElementById('image_route').value,
          video_route: document.getElementById('video_route').value,
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
        },
      };
      const imageCloud = mediaPatch('image_cloud');
      const imageEdge = mediaPatch('image_edge', {
        workflow: 'image_edge_workflow',
        workflowAlt: 'image_edge_workflow_i2i',
        workflowAltKey: 'workflow_file_i2i',
      });
      const videoCloud = mediaPatch('video_cloud');
      const videoEdge = mediaPatch('video_edge', {
        workflow: 'video_edge_workflow',
        workflowAlt: 'video_edge_workflow_i2v',
        workflowAltKey: 'workflow_file_i2v',
      });
      if (imageCloud) body.image_cloud = imageCloud;
      if (imageEdge) body.image_edge = imageEdge;
      if (videoCloud) body.video_cloud = videoCloud;
      if (videoEdge) body.video_edge = videoEdge;
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
