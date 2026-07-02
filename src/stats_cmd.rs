use std::path::PathBuf;

use anyhow::{Context, Result};
use crate::gateway::config::AppConfig;
use crate::gateway::classifier::{ClassifierSettings, ClassifierSnapshot, ClassifierStore};
use crate::gateway::experience::{ExperienceSettings, ExperienceSnapshot, ExperienceStore};
use crate::gateway::routing::compute_effective_routing;
use crate::gateway::stats::{self, StatsScope};
use serde::{Deserialize, Serialize};

use crate::client::GatewayClient;
use crate::cli_settings::CliSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsLang {
    En,
    Zh,
}

impl StatsLang {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "en" | "english" => Ok(Self::En),
            "zh" | "cn" | "chinese" | "zh-cn" | "zh_cn" => Ok(Self::Zh),
            other => anyhow::bail!("unsupported stats language `{other}` (use en or zh)"),
        }
    }
}

struct StatsLabels {
    title_global: &'static str,
    title_session: &'static str,
    gateway: &'static str,
    scope: &'static str,
    stats_file: &'static str,
    persisted: &'static str,
    first_record: &'static str,
    last_saved: &'static str,
    session_uptime: &'static str,
    global_note: &'static str,
    section_requests: &'static str,
    total: &'static str,
    stream: &'static str,
    non_stream: &'static str,
    rpm_global: &'static str,
    rpm_session: &'static str,
    section_routing: &'static str,
    edge: &'static str,
    cloud: &'static str,
    cascade: &'static str,
    section_upstream: &'static str,
    upstream_edge: &'static str,
    upstream_cloud: &'static str,
    section_cascade: &'static str,
    cascade_edge_ok: &'static str,
    cascade_fallback: &'static str,
    section_token_est: &'static str,
    tok_in: &'static str,
    tok_out: &'static str,
    cloud_input_saved: &'static str,
    section_upstream_tokens: &'static str,
    tier_edge: &'static str,
    tier_cloud: &'static str,
    tier_total: &'static str,
    edge_share: &'static str,
    cloud_share: &'static str,
    section_cloud_saved: &'static str,
    input_saved: &'static str,
    output_saved: &'static str,
    total_saved: &'static str,
    pct_would_be_cloud: &'static str,
    section_cache: &'static str,
    cache_hits: &'static str,
    cached_tokens: &'static str,
    hit_rate: &'static str,
    section_latency: &'static str,
    avg_request: &'static str,
    avg_ttft: &'static str,
    avg_tps: &'static str,
    stream_avg: &'static str,
    non_stream_avg: &'static str,
    section_served: &'static str,
    served_edge: &'static str,
    served_cloud: &'static str,
    section_difficulty: &'static str,
    avg: &'static str,
    samples: &'static str,
    section_errors: &'static str,
    unauthorized: &'static str,
    unavailable: &'static str,
    upstream_err: &'static str,
    bad_request: &'static str,
    section_step_kinds: &'static str,
    section_experience: &'static str,
    section_classifier: &'static str,
    classifier_enabled: &'static str,
    classifier_file: &'static str,
    classifier_total_samples: &'static str,
    classifier_total_updates: &'static str,
    classifier_last_retrain: &'static str,
    classifier_top_cloud: &'static str,
    experience_enabled: &'static str,
    experience_file: &'static str,
    exp_last_updated: &'static str,
    exp_sub_settings: &'static str,
    exp_learning_rate: &'static str,
    exp_max_bias: &'static str,
    exp_target_fallback: &'static str,
    exp_min_trust_samples: &'static str,
    exp_sub_totals: &'static str,
    exp_step_kinds: &'static str,
    exp_edge_ok: &'static str,
    exp_cascade_fallback: &'static str,
    exp_upstream_error: &'static str,
    exp_verified_total: &'static str,
    exp_total_outcomes: &'static str,
    exp_fallback_rate: &'static str,
    exp_edge_success_rate: &'static str,
    exp_trusted_steps: &'static str,
    exp_sub_steps: &'static str,
    exp_trusted_yes: &'static str,
    exp_trusted_no: &'static str,
    section_adaptive: &'static str,
    adaptive_enabled: &'static str,
    adaptive_verify_rate: &'static str,
    adaptive_verify_base: &'static str,
    adaptive_theta_edge: &'static str,
    adaptive_theta_edge_base: &'static str,
    adaptive_theta_cloud: &'static str,
    adaptive_theta_cloud_base: &'static str,
    adaptive_reasons: &'static str,
    section_agent_budgets: &'static str,
    budget_agent: &'static str,
    budget_limit: &'static str,
    budget_used: &'static str,
    budget_pct: &'static str,
    tier_in: &'static str,
    tier_out: &'static str,
    tier_cached: &'static str,
    scope_global: &'static str,
    scope_session: &'static str,
}

fn labels(lang: StatsLang) -> StatsLabels {
    match lang {
        StatsLang::En => StatsLabels {
            title_global: "Token Router Gateway Stats (global / all-time)",
            title_session: "Token Router Gateway Stats (current session)",
            gateway: "Gateway",
            scope: "Scope",
            stats_file: "Stats file",
            persisted: "Persisted",
            first_record: "First record (unix)",
            last_saved: "Last saved (unix)",
            session_uptime: "Session uptime",
            global_note: "(counters include all gateway runs; persisted in stats.db)",
            section_requests: "Requests",
            total: "total",
            stream: "stream",
            non_stream: "non-stream",
            rpm_global: "per minute (since first record)",
            rpm_session: "per minute (this session)",
            section_routing: "Routing decisions",
            edge: "edge",
            cloud: "cloud",
            cascade: "cascade",
            section_upstream: "Upstream HTTP calls",
            upstream_edge: "edge",
            upstream_cloud: "cloud",
            section_cascade: "Cascade (non-stream)",
            cascade_edge_ok: "edge quality pass",
            cascade_fallback: "fallback to cloud",
            section_token_est: "Token estimates (routing, cumulative)",
            tok_in: "input (tok_in)",
            tok_out: "output (tok_out)",
            cloud_input_saved: "cloud_input_saved",
            section_upstream_tokens: "Upstream tokens (actual usage)",
            tier_edge: "edge",
            tier_cloud: "cloud",
            tier_total: "total",
            edge_share: "edge share",
            cloud_share: "cloud share",
            section_cloud_saved: "Cloud tokens saved (edge served to client)",
            input_saved: "input saved",
            output_saved: "output saved",
            total_saved: "total saved",
            pct_would_be_cloud: "% of would-be-cloud",
            section_cache: "Prompt cache",
            cache_hits: "hit requests",
            cached_tokens: "cached tokens",
            hit_rate: "hit rate",
            section_latency: "Latency & throughput",
            avg_request: "avg request",
            avg_ttft: "avg TTFT (stream)",
            avg_tps: "avg TPS",
            stream_avg: "stream avg",
            non_stream_avg: "non-stream avg",
            section_served: "Responses served",
            served_edge: "edge responses",
            served_cloud: "cloud responses",
            section_difficulty: "Difficulty",
            avg: "avg",
            samples: "samples",
            section_errors: "Errors",
            unauthorized: "unauthorized",
            unavailable: "unavailable",
            upstream_err: "upstream",
            bad_request: "bad_request",
            section_step_kinds: "Step kinds",
            section_experience: "Experience (routing learning)",
            section_classifier: "Bayesian classifier",
            classifier_enabled: "enabled",
            classifier_file: "file",
            classifier_total_samples: "total samples",
            classifier_total_updates: "labeled updates",
            classifier_last_retrain: "last retrain (unix)",
            classifier_top_cloud: "top cloud-driving features",
            experience_enabled: "enabled",
            experience_file: "file",
            exp_last_updated: "last updated (unix)",
            exp_sub_settings: "Learning parameters",
            exp_learning_rate: "learning rate",
            exp_max_bias: "max bias",
            exp_target_fallback: "target fallback",
            exp_min_trust_samples: "min trust samples",
            exp_sub_totals: "Totals",
            exp_step_kinds: "step kinds tracked",
            exp_edge_ok: "edge verified ok",
            exp_cascade_fallback: "cascade fallback",
            exp_upstream_error: "upstream errors",
            exp_verified_total: "cloud-verify samples",
            exp_total_outcomes: "total outcomes",
            exp_fallback_rate: "fallback rate",
            exp_edge_success_rate: "edge success rate",
            exp_trusted_steps: "edge-trusted steps",
            exp_sub_steps: "Per step_kind",
            exp_trusted_yes: "yes",
            exp_trusted_no: "no",
            section_adaptive: "Adaptive routing (runtime)",
            adaptive_enabled: "enabled",
            adaptive_verify_rate: "work verify sample rate",
            adaptive_verify_base: "config baseline",
            adaptive_theta_edge: "θ_edge (effective)",
            adaptive_theta_edge_base: "θ_edge (config)",
            adaptive_theta_cloud: "θ_cloud (effective)",
            adaptive_theta_cloud_base: "θ_cloud (config)",
            adaptive_reasons: "reason codes",
            section_agent_budgets: "Agent cloud token budgets (5 h window)",
            budget_agent: "agent",
            budget_limit: "limit",
            budget_used: "used",
            budget_pct: "%",
            tier_in: "in",
            tier_out: "out",
            tier_cached: "cached",
            scope_global: "global",
            scope_session: "session",
        },
        StatsLang::Zh => StatsLabels {
            title_global: "Token Router 网关统计（全局 / 累计）",
            title_session: "Token Router 网关统计（当前会话）",
            gateway: "网关",
            scope: "范围",
            stats_file: "统计文件",
            persisted: "已持久化",
            first_record: "首次记录 (unix)",
            last_saved: "最后保存 (unix)",
            session_uptime: "会话运行时长",
            global_note: "（计数包含所有网关运行记录，写入 stats.db）",
            section_requests: "请求",
            total: "总计",
            stream: "流式",
            non_stream: "非流式",
            rpm_global: "每分钟（自首次记录）",
            rpm_session: "每分钟（本会话）",
            section_routing: "路由决策",
            edge: "端侧",
            cloud: "云端",
            cascade: "级联",
            section_upstream: "上游 HTTP 调用",
            upstream_edge: "端侧",
            upstream_cloud: "云端",
            section_cascade: "级联（非流式）",
            cascade_edge_ok: "端侧质量通过",
            cascade_fallback: "回退至云端",
            section_token_est: "Token 估算（路由，累计）",
            tok_in: "输入 (tok_in)",
            tok_out: "输出 (tok_out)",
            cloud_input_saved: "云端输入节省",
            section_upstream_tokens: "上游 Token（实际用量）",
            tier_edge: "端侧",
            tier_cloud: "云端",
            tier_total: "合计",
            edge_share: "端侧占比",
            cloud_share: "云端占比",
            section_cloud_saved: "节省的云端 Token（端侧响应客户端）",
            input_saved: "输入节省",
            output_saved: "输出节省",
            total_saved: "总计节省",
            pct_would_be_cloud: "占潜云端用量",
            section_cache: "Prompt 缓存",
            cache_hits: "命中请求",
            cached_tokens: "缓存 token",
            hit_rate: "命中率",
            section_latency: "延迟与吞吐",
            avg_request: "平均请求",
            avg_ttft: "平均 TTFT（流式）",
            avg_tps: "平均 TPS",
            stream_avg: "流式平均",
            non_stream_avg: "非流式平均",
            section_served: "响应来源",
            served_edge: "端侧响应",
            served_cloud: "云端响应",
            section_difficulty: "难度",
            avg: "平均",
            samples: "样本数",
            section_errors: "错误",
            unauthorized: "未授权",
            unavailable: "不可用",
            upstream_err: "上游错误",
            bad_request: "错误请求",
            section_step_kinds: "步骤类型",
            section_experience: "经验学习（路由）",
            section_classifier: "贝叶斯分类器",
            classifier_enabled: "已启用",
            classifier_file: "文件",
            classifier_total_samples: "总样本",
            classifier_total_updates: "标注更新",
            classifier_last_retrain: "上次重训 (unix)",
            classifier_top_cloud: "升云特征 Top",
            experience_enabled: "已启用",
            experience_file: "文件",
            exp_last_updated: "最后更新 (unix)",
            exp_sub_settings: "学习参数",
            exp_learning_rate: "学习率",
            exp_max_bias: "偏置上限",
            exp_target_fallback: "目标回退率",
            exp_min_trust_samples: "信任最小样本",
            exp_sub_totals: "累计",
            exp_step_kinds: "步态种类",
            exp_edge_ok: "端侧验证通过",
            exp_cascade_fallback: "回退云端",
            exp_upstream_error: "上游错误",
            exp_verified_total: "云验证样本",
            exp_total_outcomes: "总样本",
            exp_fallback_rate: "回退率",
            exp_edge_success_rate: "端侧成功率",
            exp_trusted_steps: "可直走端侧",
            exp_sub_steps: "步态明细",
            exp_trusted_yes: "是",
            exp_trusted_no: "否",
            section_adaptive: "自适应路由（运行时）",
            adaptive_enabled: "已启用",
            adaptive_verify_rate: "work 校验抽样率",
            adaptive_verify_base: "配置基线",
            adaptive_theta_edge: "θ_edge（生效）",
            adaptive_theta_edge_base: "θ_edge（配置）",
            adaptive_theta_cloud: "θ_cloud（生效）",
            adaptive_theta_cloud_base: "θ_cloud（配置）",
            adaptive_reasons: "调整原因",
            section_agent_budgets: "Agent 云端 Token 预算（5 小时窗口）",
            budget_agent: "Agent",
            budget_limit: "限额",
            budget_used: "已用",
            budget_pct: "%",
            tier_in: "输入",
            tier_out: "输出",
            tier_cached: "缓存",
            scope_global: "全局",
            scope_session: "会话",
        },
    }
}

fn scope_display(scope: &str, lang: StatsLang) -> &str {
    match (scope, lang) {
        ("global", StatsLang::Zh) => "全局",
        ("session", StatsLang::Zh) => "会话",
        (s, _) => s,
    }
}

#[cfg(test)]
mod lang_tests {
    use super::StatsLang;

    #[test]
    fn parses_lang_aliases() {
        assert_eq!(StatsLang::parse("en").unwrap(), StatsLang::En);
        assert_eq!(StatsLang::parse("zh").unwrap(), StatsLang::Zh);
        assert_eq!(StatsLang::parse("zh-cn").unwrap(), StatsLang::Zh);
        assert!(StatsLang::parse("fr").is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayStats {
    pub scope: String,
    pub stats_file: String,
    pub persisted: bool,
    #[serde(default)]
    pub first_record_at_unix: Option<u64>,
    #[serde(default)]
    pub last_updated_at_unix: Option<u64>,
    pub session_uptime_secs: u64,
    pub requests_total: u64,
    pub requests_stream: u64,
    pub requests_non_stream: u64,
    pub requests_per_minute: f64,
    pub routing: RouteCounts,
    pub upstream: UpstreamCounts,
    pub cascade: CascadeCounts,
    pub tokens: TokenCounts,
    pub token_breakdown: TokenBreakdownSection,
    pub cache: CacheSection,
    pub latency: LatencySection,
    pub served: ServedSection,
    pub difficulty: DifficultyStats,
    pub errors: ErrorCounts,
    pub step_kinds: std::collections::HashMap<String, u64>,
    pub experience: Option<ExperienceSection>,
    pub classifier: Option<ClassifierSection>,
    pub effective_routing: Option<AdaptiveRoutingSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_budgets: Option<Vec<AgentBudgetRow>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBudgetRow {
    pub agent_id: String,
    pub budget_limit: Option<u64>,
    pub tokens_used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorCounts {
    pub edge_ok: u64,
    pub cloud_needed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierSection {
    pub enabled: bool,
    pub classifier_file: String,
    pub last_updated_at_unix: Option<u64>,
    pub last_retrain_at_unix: Option<u64>,
    pub total_samples: u64,
    pub total_updates: u64,
    pub decay_generation: u64,
    pub min_samples: u64,
    pub prior_alpha: f32,
    pub decay_half_life_hours: f64,
    pub prior: PriorCounts,
    pub top_cloud_features: Vec<ClassifierFeatureRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierFeatureRow {
    pub feature: String,
    pub edge_ok: u64,
    pub cloud_needed: u64,
    pub cloud_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveRoutingSection {
    pub enabled: bool,
    pub work_verify_sample_rate: f32,
    pub theta_edge: f32,
    pub theta_cloud: f32,
    pub base_verify_sample_rate: f32,
    pub base_theta_edge: f32,
    pub base_theta_cloud: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceSection {
    pub enabled: bool,
    pub experience_file: String,
    pub last_updated_at_unix: Option<u64>,
    pub settings: ExperienceSettingsSection,
    pub totals: ExperienceTotalsSection,
    pub steps: Vec<ExperienceStepRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceSettingsSection {
    pub learning_rate: f32,
    pub max_bias: f32,
    pub target_fallback: f32,
    pub min_trust_samples: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceTotalsSection {
    pub step_kinds: u64,
    pub edge_ok: u64,
    pub cascade_fallback: u64,
    pub upstream_error: u64,
    pub verified_total: u64,
    pub total_outcomes: u64,
    pub fallback_rate: f64,
    pub edge_success_rate: f64,
    pub trusted_steps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceStepRow {
    pub step_kind: String,
    pub edge_ok: u64,
    pub cascade_fallback: u64,
    pub upstream_error: u64,
    pub verified_total: u64,
    pub total_outcomes: u64,
    pub fallback_rate: f64,
    pub edge_success_rate: f64,
    pub bias: f32,
    pub edge_trusted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteCounts {
    pub edge: u64,
    pub cloud: u64,
    pub cascade: u64,
    pub edge_pct: f64,
    pub cloud_pct: f64,
    pub cascade_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamCounts {
    pub edge_calls: u64,
    pub cloud_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeCounts {
    pub edge_ok: u64,
    pub fallback_to_cloud: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCounts {
    pub in_estimate: u64,
    pub out_estimate: u64,
    pub cloud_input_saved_estimate: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBreakdownSection {
    pub edge: TierTokenSection,
    pub cloud: TierTokenSection,
    pub total: TierTokenSection,
    pub edge_share_pct: f64,
    pub cloud_share_pct: f64,
    pub cloud_saved: CloudTokensSavedSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierTokenSection {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    pub input_pct: f64,
    pub output_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTokensSavedSection {
    pub input: u64,
    pub output: u64,
    pub total: u64,
    pub pct_of_would_be_cloud: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheSection {
    pub hit_requests: u64,
    pub cached_tokens: u64,
    pub hit_rate_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencySection {
    pub avg_request_ms: f64,
    pub avg_ttft_ms: f64,
    pub avg_tps: f64,
    pub edge_tps: f64,
    pub cloud_tps: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub upstream_samples: u64,
    pub ttft_samples: u64,
    pub tps_samples: u64,
    pub stream_avg_ms: f64,
    pub non_stream_avg_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServedSection {
    pub edge: u64,
    pub cloud: u64,
    pub edge_pct: f64,
    pub cloud_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifficultyStats {
    pub avg: f64,
    pub samples: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorCounts {
    pub total: u64,
    pub unauthorized: u64,
    pub unavailable: u64,
    pub upstream: u64,
    pub bad_request: u64,
}

pub async fn print_stats(
    config_override: &Option<PathBuf>,
    global: bool,
    json: bool,
    lang: &str,
) -> Result<()> {
    let lang = StatsLang::parse(lang)?;
    let settings = load_settings(config_override)?;

    let stats = if global {
        load_global_stats(&settings).await?
    } else {
        let client = GatewayClient::new(
            settings.gateway_url(),
            settings.api_key(),
            settings.admin_token(),
        );
        client
            .stats_session()
            .await
            .context("fetch session stats (is `token-router gateway start` running?)")?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        print_human(&stats, &settings.gateway_url(), lang);
    }
    Ok(())
}

async fn load_global_stats(settings: &CliSettings) -> Result<GatewayStats> {
    let client = GatewayClient::new(
        settings.gateway_url(),
        settings.api_key(),
        settings.admin_token(),
    );
    if let Ok(stats) = client.stats_global().await {
        return Ok(stats);
    }

    load_global_stats_from_disk(settings)
}

fn load_global_stats_from_disk(settings: &CliSettings) -> Result<GatewayStats> {
    let data_dir = crate::config::app_dir()?;
    let db_path = crate::config::stats_db()?;
    let db_exists = db_path.exists();
    let stats_path = if db_exists {
        db_path.clone()
    } else {
        crate::config::stats_file()?
    };
    let data = if db_exists {
        let db = stats::db::StatsDb::open(&data_dir, &data_dir.join("stats.json.bak"))?;
        db.load_totals(StatsScope::Global)?
    } else {
        stats::data::load(&stats_path)?
    };
    let latency_p95_p99 = if db_exists {
        stats::db::StatsDb::open(&data_dir, &data_dir.join("stats.json.bak"))?
            .latency_percentiles(StatsScope::Global)
            .ok()
    } else {
        None
    };
    let experience = experience_snapshot_from_disk(&data_dir, settings).ok();
    let classifier = classifier_snapshot_from_disk(&data_dir, settings).ok();
    let effective_routing = experience.as_ref().and_then(|exp| {
        let config = AppConfig::from_file(settings.file.clone(), settings.config_path.clone()).ok()?;
        Some(compute_effective_routing(
            &config,
            exp,
            Some(&data),
            &config.adaptive_routing,
        ))
    });
    let mut snap = stats::build_snapshot(
        &data,
        StatsScope::Global,
        stats_path.display().to_string(),
        0,
        experience,
        classifier,
        effective_routing,
        None,
    );
    if let Some((p95, p99)) = latency_p95_p99 {
        snap.latency.p95_ms = p95;
        snap.latency.p99_ms = p99;
    }
    Ok(from_snapshot(snap))
}

fn experience_snapshot_from_disk(
    data_dir: &std::path::Path,
    settings: &CliSettings,
) -> Result<ExperienceSnapshot> {
    let gw = settings.file.gateway.clone();
    let store = ExperienceStore::open(
        data_dir,
        ExperienceSettings {
            enabled: gw.experience_enabled,
            learning_rate: gw.experience_learning_rate,
            max_bias: gw.experience_max_bias,
            target_fallback: gw.experience_target_fallback,
        },
    )?;
    Ok(store.snapshot())
}

fn classifier_snapshot_from_disk(
    data_dir: &std::path::Path,
    settings: &CliSettings,
) -> Result<ClassifierSnapshot> {
    let gw = settings.file.gateway.clone();
    let store = ClassifierStore::open(
        data_dir,
        ClassifierSettings {
            enabled: gw.classifier_enabled,
            min_samples: gw.classifier_min_samples,
            prior_alpha: gw.classifier_prior_alpha,
            decay_half_life_hours: gw.classifier_decay_half_life_hours,
            prior_from_heuristic: gw.classifier_prior_from_heuristic,
            min_feature_count: 0.5,
        },
    )?;
    Ok(store.snapshot())
}

fn from_snapshot(s: stats::StatsSnapshot) -> GatewayStats {
    GatewayStats {
        scope: s.scope,
        stats_file: s.stats_file,
        persisted: s.persisted,
        first_record_at_unix: s.first_record_at_unix,
        last_updated_at_unix: s.last_updated_at_unix,
        session_uptime_secs: s.session_uptime_secs,
        requests_total: s.requests_total,
        requests_stream: s.requests_stream,
        requests_non_stream: s.requests_non_stream,
        requests_per_minute: s.requests_per_minute,
        routing: RouteCounts {
            edge: s.routing.edge,
            cloud: s.routing.cloud,
            cascade: s.routing.cascade,
            edge_pct: s.routing.edge_pct,
            cloud_pct: s.routing.cloud_pct,
            cascade_pct: s.routing.cascade_pct,
        },
        upstream: UpstreamCounts {
            edge_calls: s.upstream.edge_calls,
            cloud_calls: s.upstream.cloud_calls,
        },
        cascade: CascadeCounts {
            edge_ok: s.cascade.edge_ok,
            fallback_to_cloud: s.cascade.fallback_to_cloud,
        },
        tokens: TokenCounts {
            in_estimate: s.tokens.in_estimate,
            out_estimate: s.tokens.out_estimate,
            cloud_input_saved_estimate: s.tokens.cloud_input_saved_estimate,
        },
        token_breakdown: TokenBreakdownSection {
            edge: tier_from(&s.token_breakdown.edge),
            cloud: tier_from(&s.token_breakdown.cloud),
            total: tier_from(&s.token_breakdown.total),
            edge_share_pct: s.token_breakdown.edge_share_pct,
            cloud_share_pct: s.token_breakdown.cloud_share_pct,
            cloud_saved: CloudTokensSavedSection {
                input: s.token_breakdown.cloud_saved.input,
                output: s.token_breakdown.cloud_saved.output,
                total: s.token_breakdown.cloud_saved.total,
                pct_of_would_be_cloud: s.token_breakdown.cloud_saved.pct_of_would_be_cloud,
            },
        },
        cache: CacheSection {
            hit_requests: s.cache.hit_requests,
            cached_tokens: s.cache.cached_tokens,
            hit_rate_pct: s.cache.hit_rate_pct,
        },
        latency: LatencySection {
            avg_request_ms: s.latency.avg_request_ms,
            avg_ttft_ms: s.latency.avg_ttft_ms,
            avg_tps: s.latency.avg_tps,
            edge_tps: s.latency.edge_tps,
            cloud_tps: s.latency.cloud_tps,
            p95_ms: s.latency.p95_ms,
            p99_ms: s.latency.p99_ms,
            upstream_samples: s.latency.upstream_samples,
            ttft_samples: s.latency.ttft_samples,
            tps_samples: s.latency.tps_samples,
            stream_avg_ms: s.latency.stream_avg_ms,
            non_stream_avg_ms: s.latency.non_stream_avg_ms,
        },
        served: ServedSection {
            edge: s.served.edge,
            cloud: s.served.cloud,
            edge_pct: s.served.edge_pct,
            cloud_pct: s.served.cloud_pct,
        },
        difficulty: DifficultyStats {
            avg: s.difficulty.avg,
            samples: s.difficulty.samples,
        },
        errors: ErrorCounts {
            total: s.errors.total,
            unauthorized: s.errors.unauthorized,
            unavailable: s.errors.unavailable,
            upstream: s.errors.upstream,
            bad_request: s.errors.bad_request,
        },
        step_kinds: s.step_kinds,
        experience: s.experience.map(experience_from_snapshot),
        classifier: s.classifier.map(classifier_from_snapshot),
        effective_routing: s.effective_routing.map(adaptive_from_snapshot),
        agent_budgets: s.agent_budgets.map(|b| {
            b.into_iter()
                .map(|r| AgentBudgetRow {
                    agent_id: r.agent_id,
                    budget_limit: r.budget_limit,
                    tokens_used: r.tokens_used,
                })
                .collect()
        }),
    }
}

fn classifier_from_snapshot(c: ClassifierSnapshot) -> ClassifierSection {
    ClassifierSection {
        enabled: c.enabled,
        classifier_file: c.classifier_file,
        last_updated_at_unix: c.last_updated_at_unix,
        last_retrain_at_unix: c.last_retrain_at_unix,
        total_samples: c.total_samples,
        total_updates: c.total_updates,
        decay_generation: c.decay_generation,
        min_samples: c.min_samples,
        prior_alpha: c.prior_alpha,
        decay_half_life_hours: c.decay_half_life_hours,
        prior: PriorCounts {
            edge_ok: c.prior.edge_ok,
            cloud_needed: c.prior.cloud_needed,
        },
        top_cloud_features: c
            .top_cloud_features
            .into_iter()
            .map(|f| ClassifierFeatureRow {
                feature: f.feature,
                edge_ok: f.edge_ok,
                cloud_needed: f.cloud_needed,
                cloud_rate: f.cloud_rate,
            })
            .collect(),
    }
}

fn adaptive_from_snapshot(r: stats::EffectiveRouting) -> AdaptiveRoutingSection {
    AdaptiveRoutingSection {
        enabled: r.enabled,
        work_verify_sample_rate: r.work_verify_sample_rate,
        theta_edge: r.theta_edge,
        theta_cloud: r.theta_cloud,
        base_verify_sample_rate: r.base_verify_sample_rate,
        base_theta_edge: r.base_theta_edge,
        base_theta_cloud: r.base_theta_cloud,
        reasons: r.reasons,
    }
}

fn experience_from_snapshot(exp: ExperienceSnapshot) -> ExperienceSection {
    ExperienceSection {
        enabled: exp.enabled,
        experience_file: exp.experience_file,
        last_updated_at_unix: exp.last_updated_at_unix,
        settings: ExperienceSettingsSection {
            learning_rate: exp.settings.learning_rate,
            max_bias: exp.settings.max_bias,
            target_fallback: exp.settings.target_fallback,
            min_trust_samples: exp.settings.min_trust_samples,
        },
        totals: ExperienceTotalsSection {
            step_kinds: exp.totals.step_kinds,
            edge_ok: exp.totals.edge_ok,
            cascade_fallback: exp.totals.cascade_fallback,
            upstream_error: exp.totals.upstream_error,
            verified_total: exp.totals.verified_total,
            total_outcomes: exp.totals.total_outcomes,
            fallback_rate: exp.totals.fallback_rate,
            edge_success_rate: exp.totals.edge_success_rate,
            trusted_steps: exp.totals.trusted_steps,
        },
        steps: exp
            .steps
            .into_iter()
            .map(|row| ExperienceStepRow {
                step_kind: row.step_kind,
                edge_ok: row.edge_ok,
                cascade_fallback: row.cascade_fallback,
                upstream_error: row.upstream_error,
                verified_total: row.verified_total,
                total_outcomes: row.total_outcomes,
                fallback_rate: row.fallback_rate,
                edge_success_rate: row.edge_success_rate,
                bias: row.bias,
                edge_trusted: row.edge_trusted,
            })
            .collect(),
    }
}

fn tier_from(t: &stats::TierTokenStats) -> TierTokenSection {
    TierTokenSection {
        input: t.input,
        output: t.output,
        cached: t.cached,
        input_pct: t.input_pct,
        output_pct: t.output_pct,
    }
}

fn load_settings(config_override: &Option<PathBuf>) -> Result<CliSettings> {
    let path = match config_override {
        Some(p) => p.clone(),
        None => crate::config::config_file()?,
    };
    let (file, config_path) = crate::config::load_from_path(&path)?;
    Ok(CliSettings { file, config_path })
}

// ── Human-readable layout ─────────────────────────────────────────────

const BAR_WIDTH: usize = 12;
const SECTION_RULE: &str = "────────────────────────────────────────────────────────";

struct Fmt {
    indent: &'static str,
    label_w: usize,
}

impl Fmt {
    fn for_lang(lang: StatsLang) -> Self {
        Self {
            indent: "  ",
            label_w: match lang {
                StatsLang::En => 22,
                StatsLang::Zh => 14,
            },
        }
    }

    fn banner(&self, title: &str) {
        println!();
        println!("{title}");
        println!("{SECTION_RULE}");
    }

    fn section(&self, title: &str) {
        println!();
        println!("{title}");
        println!("{SECTION_RULE}");
    }

    fn kv(&self, label: &str, value: impl std::fmt::Display) {
        println!(
            "{}{:<label_w$}  {value}",
            self.indent,
            label,
            label_w = self.label_w
        );
    }

    fn kv_note(&self, text: &str) {
        println!("{}{text}", self.indent);
    }

    fn kv_pct(&self, label: &str, count: u64, pct: f64) {
        println!(
            "{}{:<label_w$}  {:>8}  {}  {:>5.1}%",
            self.indent,
            label,
            fmt_u(count),
            pct_bar(pct, BAR_WIDTH),
            pct,
            label_w = self.label_w
        );
    }

    fn kv_pct_only(&self, label: &str, pct: f64) {
        println!(
            "{}{:<label_w$}  {}  {:>5.1}%",
            self.indent,
            label,
            pct_bar(pct, BAR_WIDTH),
            pct,
            label_w = self.label_w
        );
    }

    fn kv_f64(&self, label: &str, value: f64, decimals: usize, unit: &str) {
        println!(
            "{}{:<label_w$}  {value:.prec$}{unit}",
            self.indent,
            label,
            value = value,
            prec = decimals,
            unit = unit,
            label_w = self.label_w
        );
    }

    fn kv_f64_n(&self, label: &str, value: f64, decimals: usize, unit: &str, n: u64) {
        println!(
            "{}{:<label_w$}  {value:.prec$}{unit}  (n={n})",
            self.indent,
            label,
            value = value,
            prec = decimals,
            unit = unit,
            label_w = self.label_w
        );
    }
}

fn fmt_u(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn pct_bar(pct: f64, width: usize) -> String {
    let clamped = pct.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "·".repeat(width - filled))
}

fn print_human(s: &GatewayStats, gateway_url: &str, lang: StatsLang) {
    let l = labels(lang);
    let f = Fmt::for_lang(lang);
    let title = if s.scope == "global" {
        l.title_global
    } else {
        l.title_session
    };

    f.banner(title);
    f.kv(l.gateway, gateway_url);
    f.kv(l.scope, scope_display(&s.scope, lang));
    f.kv(l.stats_file, &s.stats_file);
    f.kv(l.persisted, s.persisted);
    if let Some(ts) = s.first_record_at_unix {
        f.kv(l.first_record, ts);
    }
    if let Some(ts) = s.last_updated_at_unix {
        f.kv(l.last_saved, ts);
    }
    if s.scope == "session" {
        f.kv(l.session_uptime, format!("{}s", s.session_uptime_secs));
    }
    if s.scope == "global" {
        f.kv_note(l.global_note);
    }

    f.section(l.section_requests);
    f.kv(l.total, fmt_u(s.requests_total));
    f.kv(l.stream, fmt_u(s.requests_stream));
    f.kv(l.non_stream, fmt_u(s.requests_non_stream));
    let rpm_label = if s.scope == "global" {
        l.rpm_global
    } else {
        l.rpm_session
    };
    f.kv_f64(rpm_label, s.requests_per_minute, 2, "");

    f.section(l.section_routing);
    f.kv_pct(l.edge, s.routing.edge, s.routing.edge_pct);
    f.kv_pct(l.cloud, s.routing.cloud, s.routing.cloud_pct);
    f.kv_pct(l.cascade, s.routing.cascade, s.routing.cascade_pct);
    f.kv(l.upstream_edge, fmt_u(s.upstream.edge_calls));
    f.kv(l.upstream_cloud, fmt_u(s.upstream.cloud_calls));
    f.kv(l.cascade_edge_ok, fmt_u(s.cascade.edge_ok));
    f.kv(l.cascade_fallback, fmt_u(s.cascade.fallback_to_cloud));
    f.kv_pct(l.served_edge, s.served.edge, s.served.edge_pct);
    f.kv_pct(l.served_cloud, s.served.cloud, s.served.cloud_pct);

    f.section(l.section_token_est);
    f.kv(l.tok_in, fmt_u(s.tokens.in_estimate));
    f.kv(l.tok_out, fmt_u(s.tokens.out_estimate));
    f.kv(l.cloud_input_saved, fmt_u(s.tokens.cloud_input_saved_estimate));

    print_token_table(&f, &l, s);

    f.section(l.section_cloud_saved);
    f.kv(l.input_saved, fmt_u(s.token_breakdown.cloud_saved.input));
    f.kv(l.output_saved, fmt_u(s.token_breakdown.cloud_saved.output));
    f.kv(l.total_saved, fmt_u(s.token_breakdown.cloud_saved.total));
    f.kv_f64(
        l.pct_would_be_cloud,
        s.token_breakdown.cloud_saved.pct_of_would_be_cloud,
        1,
        "%",
    );

    f.section(l.section_cache);
    f.kv(l.cache_hits, fmt_u(s.cache.hit_requests));
    f.kv(l.cached_tokens, fmt_u(s.cache.cached_tokens));
    f.kv_f64(l.hit_rate, s.cache.hit_rate_pct, 1, "%");

    f.section(l.section_latency);
    f.kv_f64_n(
        l.avg_request,
        s.latency.avg_request_ms,
        1,
        " ms",
        s.latency.upstream_samples,
    );
    f.kv_f64_n(l.avg_ttft, s.latency.avg_ttft_ms, 1, " ms", s.latency.ttft_samples);
    f.kv_f64_n(l.avg_tps, s.latency.avg_tps, 1, " tok/s", s.latency.tps_samples);
    f.kv_f64(l.stream_avg, s.latency.stream_avg_ms, 1, " ms");
    f.kv_f64(l.non_stream_avg, s.latency.non_stream_avg_ms, 1, " ms");

    f.section(l.section_difficulty);
    f.kv_f64(l.avg, s.difficulty.avg, 3, "");
    f.kv(l.samples, fmt_u(s.difficulty.samples));

    f.section(l.section_errors);
    f.kv(l.total, fmt_u(s.errors.total));
    if s.errors.total > 0 {
        f.kv(l.unauthorized, fmt_u(s.errors.unauthorized));
        f.kv(l.unavailable, fmt_u(s.errors.unavailable));
        f.kv(l.upstream_err, fmt_u(s.errors.upstream));
        f.kv(l.bad_request, fmt_u(s.errors.bad_request));
    }

    if !s.step_kinds.is_empty() {
        print_step_kinds(&f, &l, s);
    }
    if let Some(exp) = &s.experience {
        print_experience(&f, &l, exp, lang);
    }
    if let Some(clf) = &s.classifier {
        print_classifier(&f, &l, clf);
    }
    if let Some(adaptive) = &s.effective_routing {
        print_adaptive(&f, &l, adaptive);
    }
    if let Some(budgets) = &s.agent_budgets {
        if !budgets.is_empty() {
            print_agent_budgets(&f, &l, budgets);
        }
    }
    println!();
}

fn print_token_table(f: &Fmt, l: &StatsLabels, s: &GatewayStats) {
    f.section(l.section_upstream_tokens);
    let col = if f.label_w >= 20 { 12 } else { 10 };
    println!(
        "{}  {:>col_w$}  {:>col_w$}  {:>col_w$}  {:>col_w$}",
        f.indent,
        "",
        l.tier_in,
        l.tier_out,
        l.tier_cached,
        col_w = col
    );
    print_token_row(f, l.tier_edge, &s.token_breakdown.edge, col);
    print_token_row(f, l.tier_cloud, &s.token_breakdown.cloud, col);
    print_token_row(f, l.tier_total, &s.token_breakdown.total, col);
    println!();
    f.kv_pct_only(l.edge_share, s.token_breakdown.edge_share_pct);
    f.kv_pct_only(l.cloud_share, s.token_breakdown.cloud_share_pct);
}

fn print_token_row(f: &Fmt, tier: &str, t: &TierTokenSection, col: usize) {
    println!(
        "{}  {:>col_w$}  {:>col_w$}  {:>col_w$}  {:>col_w$}",
        f.indent,
        tier,
        fmt_u(t.input),
        fmt_u(t.output),
        fmt_u(t.cached),
        col_w = col
    );
    println!(
        "{}  {:>col_w$}  {:>col_w$}  {:>col_w$}  {:>col_w$}",
        f.indent,
        "",
        format!("({:.1}%)", t.input_pct),
        format!("({:.1}%)", t.output_pct),
        "",
        col_w = col
    );
}

fn print_step_kinds(f: &Fmt, l: &StatsLabels, s: &GatewayStats) {
    f.section(l.section_step_kinds);
    let mut kinds: Vec<_> = s.step_kinds.iter().collect();
    kinds.sort_by(|a, b| b.1.cmp(a.1));
    let name_w = kinds
        .iter()
        .map(|(k, _)| k.len())
        .max()
        .unwrap_or(8)
        .max(8);
    for (name, count) in kinds {
        println!(
            "{}{name:<name_w$}  {:>8}",
            f.indent,
            fmt_u(*count),
            name_w = name_w
        );
    }
}

fn print_classifier(f: &Fmt, l: &StatsLabels, clf: &ClassifierSection) {
    f.section(l.section_classifier);
    f.kv(l.classifier_enabled, clf.enabled);
    f.kv(l.classifier_file, &clf.classifier_file);
    f.kv(l.classifier_total_samples, fmt_u(clf.total_samples));
    f.kv(l.classifier_total_updates, fmt_u(clf.total_updates));
    if let Some(ts) = clf.last_retrain_at_unix {
        f.kv(l.classifier_last_retrain, ts);
    }
    if clf.top_cloud_features.is_empty() {
        return;
    }
    println!();
    f.kv_note(l.classifier_top_cloud);
    for row in &clf.top_cloud_features {
        println!(
            "{}  {:<28} {:>6} {:>6} {:>6.1}%",
            f.indent,
            row.feature,
            fmt_u(row.edge_ok),
            fmt_u(row.cloud_needed),
            row.cloud_rate * 100.0
        );
    }
}

fn print_experience(f: &Fmt, l: &StatsLabels, exp: &ExperienceSection, lang: StatsLang) {
    f.section(l.section_experience);
    f.kv(l.experience_enabled, exp.enabled);
    f.kv(l.experience_file, &exp.experience_file);
    if let Some(ts) = exp.last_updated_at_unix {
        f.kv(l.exp_last_updated, ts);
    }

    println!();
    f.kv_note(l.exp_sub_settings);
    f.kv_f64(l.exp_learning_rate, exp.settings.learning_rate as f64, 3, "");
    f.kv_f64(l.exp_max_bias, exp.settings.max_bias as f64, 3, "");
    f.kv_f64(
        l.exp_target_fallback,
        exp.settings.target_fallback as f64 * 100.0,
        1,
        "%",
    );
    f.kv(l.exp_min_trust_samples, exp.settings.min_trust_samples);

    println!();
    f.kv_note(l.exp_sub_totals);
    f.kv(l.exp_step_kinds, fmt_u(exp.totals.step_kinds));
    f.kv(l.exp_edge_ok, fmt_u(exp.totals.edge_ok));
    f.kv(l.exp_cascade_fallback, fmt_u(exp.totals.cascade_fallback));
    f.kv(l.exp_upstream_error, fmt_u(exp.totals.upstream_error));
    f.kv(l.exp_verified_total, fmt_u(exp.totals.verified_total));
    f.kv(l.exp_total_outcomes, fmt_u(exp.totals.total_outcomes));
    f.kv_f64(l.exp_fallback_rate, exp.totals.fallback_rate * 100.0, 1, "%");
    f.kv_f64(
        l.exp_edge_success_rate,
        exp.totals.edge_success_rate * 100.0,
        1,
        "%",
    );
    f.kv(l.exp_trusted_steps, fmt_u(exp.totals.trusted_steps));

    if exp.steps.is_empty() {
        return;
    }

    println!();
    f.kv_note(l.exp_sub_steps);
    let headers = exp_table_headers(lang);
    println!(
        "{}  {:<16} {:>5} {:>8} {:>5} {:>6} {:>7} {:>7} {:>7} {:>5}",
        f.indent,
        headers.kind,
        headers.ok,
        headers.fallback,
        headers.err,
        headers.verify,
        headers.ok_rate,
        headers.fallback_rate,
        headers.bias,
        headers.trust
    );
    for row in &exp.steps {
        let trust = if row.edge_trusted {
            l.exp_trusted_yes
        } else {
            l.exp_trusted_no
        };
        println!(
            "{}  {:<16} {:>5} {:>8} {:>5} {:>6} {:>6}% {:>6}% {:>+7.3} {:>5}",
            f.indent,
            row.step_kind,
            row.edge_ok,
            row.cascade_fallback,
            row.upstream_error,
            row.verified_total,
            row.edge_success_rate * 100.0,
            row.fallback_rate * 100.0,
            row.bias,
            trust
        );
    }
}

fn print_adaptive(f: &Fmt, l: &StatsLabels, r: &AdaptiveRoutingSection) {
    f.section(l.section_adaptive);
    f.kv(l.adaptive_enabled, r.enabled);
    f.kv_f64(
        l.adaptive_verify_rate,
        r.work_verify_sample_rate as f64 * 100.0,
        1,
        "%",
    );
    f.kv_f64(
        l.adaptive_verify_base,
        r.base_verify_sample_rate as f64 * 100.0,
        1,
        "%",
    );
    f.kv_f64(l.adaptive_theta_edge, r.theta_edge as f64, 3, "");
    f.kv_f64(l.adaptive_theta_edge_base, r.base_theta_edge as f64, 3, "");
    f.kv_f64(l.adaptive_theta_cloud, r.theta_cloud as f64, 3, "");
    f.kv_f64(l.adaptive_theta_cloud_base, r.base_theta_cloud as f64, 3, "");
    if !r.reasons.is_empty() {
        f.kv(l.adaptive_reasons, r.reasons.join(", "));
    }
}

struct ExpTableHeaders {
    kind: &'static str,
    ok: &'static str,
    fallback: &'static str,
    err: &'static str,
    verify: &'static str,
    ok_rate: &'static str,
    fallback_rate: &'static str,
    bias: &'static str,
    trust: &'static str,
}

fn print_agent_budgets(f: &Fmt, l: &StatsLabels, budgets: &[AgentBudgetRow]) {
    f.section(l.section_agent_budgets);
    println!(
        "{}  {:<12} {:>12} {:>12} {:>6}",
        f.indent, l.budget_agent, l.budget_limit, l.budget_used, l.budget_pct,
    );
    let mut sorted: Vec<_> = budgets.iter().collect();
    sorted.sort_by(|a, b| b.budget_limit.unwrap_or(0).cmp(&a.budget_limit.unwrap_or(0)));
    for row in &sorted {
        let limit = row.budget_limit.unwrap_or(0);
        let used = row.tokens_used;
        let used_pct = if limit > 0 {
            (used as f64 * 100.0 / limit as f64).min(100.0)
        } else {
            0.0
        };
        let limit_str = if limit > 0 {
            fmt_u(limit)
        } else {
            "∞".to_string()
        };
        println!(
            "{}  {:<12} {:>12} {:>12} {:>5.0}%",
            f.indent, row.agent_id, limit_str, fmt_u(used), used_pct,
        );
    }
}

fn exp_table_headers(lang: StatsLang) -> ExpTableHeaders {
    match lang {
        StatsLang::En => ExpTableHeaders {
            kind: "step_kind",
            ok: "ok",
            fallback: "fallback",
            err: "err",
            verify: "verify",
            ok_rate: "ok%",
            fallback_rate: "fb%",
            bias: "bias",
            trust: "trust",
        },
        StatsLang::Zh => ExpTableHeaders {
            kind: "步态",
            ok: "通过",
            fallback: "回退",
            err: "错误",
            verify: "验证",
            ok_rate: "成功率",
            fallback_rate: "回退率",
            bias: "偏置",
            trust: "信任",
        },
    }
}

#[cfg(test)]
mod format_tests {
    use super::{fmt_u, pct_bar};

    #[test]
    fn fmt_u_separates_thousands() {
        assert_eq!(fmt_u(0), "0");
        assert_eq!(fmt_u(1234), "1,234");
        assert_eq!(fmt_u(1_000_000), "1,000,000");
    }

    #[test]
    fn pct_bar_clamps_and_fills() {
        assert_eq!(pct_bar(0.0, 10), "··········");
        assert_eq!(pct_bar(100.0, 10), "██████████");
        assert!(pct_bar(50.0, 10).contains('█'));
    }
}
