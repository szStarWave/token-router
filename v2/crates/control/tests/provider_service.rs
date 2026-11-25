//! Provider service 回归：provider 级停用投影到编译候选的 `enabled`。

use std::sync::Arc;

use tr_agent::config::{Catalog, ConfigFile};
use tr_agent::pipeline::Pipeline;
use tr_agent::store::{self, Db};
use tr_contracts::{
    AccountInfo, CatalogModel, CatalogSelectRequest, CompileRequest, ModelPurposeDto,
    ProviderKindDto, ProviderUpsertRequest,
};
use tr_control::AdminAuth;
use tr_control::provider_service;
use tr_control::state::ControlState;

const BASE: &str = r#"
schema_version = 1

[server]
listen = "127.0.0.1:16621"

[profiles.default]
mode = "cascade"
strength_order = ["local", "cloud"]

[[candidates]]
id = "a"
tier = "local"
strength = 10
provider = "openai"
base_url = "http://127.0.0.1:11434/v1"
model = "qwen2.5:14b"
window = 32768
input = ["text"]

[[virtual_models]]
provider = "router"
id = "auto"
name = "Auto"
profile = "default"
candidate_pool = ["a"]
"#;

struct Harness {
    _dir: tempfile::TempDir,
    state: ControlState,
    path: std::path::PathBuf,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, BASE).expect("write config");

    let catalog: Catalog = ConfigFile::parse(BASE)
        .expect("parse")
        .into_catalog()
        .expect("catalog");
    let db = Db::open_in_memory().expect("db");
    store::providers::insert_provider(
        &db,
        "p1",
        "Test Provider",
        ProviderKindDto::OpenAiCompatible,
        "http://127.0.0.1:9/v1",
        None,
        None,
    )
    .expect("insert provider");
    store::providers::insert_account(
        &db,
        &AccountInfo {
            id: "acc1".to_string(),
            provider_id: "p1".to_string(),
            label: "main".to_string(),
            credential_ref: "file:credentials/acc1.key".to_string(),
            fingerprint: "abcd".to_string(),
            proxy_url: None,
            base_url_override: None,
            extra_headers: None,
            enabled: true,
            verified_at: None,
            verify_status: None,
            created_at: 0,
        },
    )
    .expect("insert account");
    store::providers::upsert_catalog_models(
        &db,
        "p1",
        &[
            CatalogModel {
                model_id: "m1".to_string(),
                display_name: "M1".to_string(),
                selected: true,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".to_string(),
                hidden: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
            },
            CatalogModel {
                model_id: "m2".to_string(),
                display_name: "M2".to_string(),
                selected: false,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".to_string(),
                hidden: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
            },
        ],
    )
    .expect("insert catalog");

    let pipeline = Arc::new(Pipeline::from_catalog(db, catalog));
    let mut state = ControlState::new(
        pipeline,
        AdminAuth::new(Arc::new(Db::open_in_memory().expect("auth db")), None),
    );
    state = state.with_config_path(path.clone());
    Harness {
        _dir: dir,
        state,
        path,
    }
}

fn compiled_enabled(path: &std::path::Path) -> bool {
    let text = std::fs::read_to_string(path).expect("read config");
    let file = ConfigFile::parse(&text).expect("parse config");
    let candidate = file
        .candidates
        .iter()
        .find(|candidate| candidate.id.starts_with("p1-m1-"))
        .expect("compiled candidate");
    candidate.enabled
}

#[test]
fn provider_enabled_projects_to_compiled_candidate() {
    let h = harness();
    store::providers::set_provider_enabled(h.state.pipeline.db(), "p1", false).expect("disable");

    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile disabled");
    assert!(!compiled_enabled(&h.path));

    store::providers::set_provider_enabled(h.state.pipeline.db(), "p1", true).expect("enable");
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile enabled");
    assert!(compiled_enabled(&h.path));
}

#[test]
fn compile_skips_embedding_and_rerank_models() {
    let h = harness();
    store::providers::upsert_catalog_models(
        h.state.pipeline.db(),
        "p1",
        &[
            CatalogModel {
                model_id: "m1".to_string(),
                display_name: "M1".to_string(),
                selected: true,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".to_string(),
                hidden: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
            },
            CatalogModel {
                model_id: "BAAI/bge-large-en-v1.5".to_string(),
                display_name: "bge-large".to_string(),
                selected: true,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".to_string(),
                hidden: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
            },
        ],
    )
    .expect("insert embedding model");

    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");

    let text = std::fs::read_to_string(&h.path).expect("read config");
    assert!(!text.contains("bge"), "embedding 不应编译进候选: {text}");
    assert!(text.contains("p1-m1-"), "对话模型仍应编译: {text}");

    let catalog = provider_service::catalog(&h.state, "p1").expect("catalog");
    assert!(
        catalog
            .models
            .iter()
            .all(|model| !model.model_id.contains("bge")),
        "embedding 不应出现在对话模型目录"
    );
}

#[test]
fn catalog_delete_blocks_references_and_restores_unreferenced() {
    let h = harness();
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");

    // 路由组由用户手动创建（编译不建组）：手写一个引用 m1 的组来验证引用检查。
    let compiled = compiled_candidate(&h.path, "p1-m1-").id;
    add_manual_group(&h.path, "group-m1", &compiled);

    // m1 已被 group-m1 的候选池引用：删除必须 409 并指出引用。
    let error = provider_service::delete_catalog_model(&h.state, "p1", "m1")
        .expect_err("referenced model must be blocked");
    assert_eq!(error.kind, tr_control::error::ApiErrorType::ConfigConflict);
    assert!(
        error
            .message
            .contains("virtual_models.group-m1.candidate_pool"),
        "{}",
        error.message
    );

    // m2 未被引用：可以删除（持久隐藏），再恢复。
    provider_service::delete_catalog_model(&h.state, "p1", "m2").expect("delete unreferenced");
    let models =
        tr_agent::store::providers::list_catalog_models(h.state.pipeline.db(), "p1").expect("list");
    assert!(models.iter().find(|m| m.model_id == "m2").unwrap().hidden);

    provider_service::select_model(
        &h.state,
        "p1",
        CatalogSelectRequest {
            model_id: "m2".to_string(),
            selected: false,
            purpose: None,
            caps: None,
            context_window: None,
            max_output: None,
            price: None,
            tier: None,
            strength: None,
            auto_include_pattern: None,
            clear_deprecated: None,
            hidden: Some(false),
        },
    )
    .expect("restore");
    let models = tr_agent::store::providers::list_catalog_models(h.state.pipeline.db(), "p1")
        .expect("list after restore");
    assert!(!models.iter().find(|m| m.model_id == "m2").unwrap().hidden);
}

#[test]
fn judge_models_compile_without_entering_chat_pool() {
    let h = harness();
    store::providers::upsert_catalog_models(
        h.state.pipeline.db(),
        "p1",
        &[
            CatalogModel {
                model_id: "m1".to_string(),
                display_name: "M1".to_string(),
                selected: true,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".to_string(),
                hidden: false,
                purpose: ModelPurposeDto::Chat,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
            },
            CatalogModel {
                model_id: "judge-llm".to_string(),
                display_name: "Judge LLM".to_string(),
                selected: false,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".to_string(),
                hidden: false,
                purpose: ModelPurposeDto::JudgeLlm,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
            },
            CatalogModel {
                model_id: "jev".to_string(),
                display_name: "JEV".to_string(),
                selected: false,
                candidate_id: None,
                context_window: None,
                max_output: None,
                price: None,
                source: "catalog".to_string(),
                hidden: false,
                purpose: ModelPurposeDto::Jev,
                caps: Vec::new(),
                thinking_levels: Vec::new(),
            },
        ],
    )
    .expect("insert judge catalog");

    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");

    let text = std::fs::read_to_string(&h.path).expect("read config");
    let file = ConfigFile::parse(&text).expect("parse config");
    let llm = file
        .candidates
        .iter()
        .find(|candidate| candidate.id.starts_with("p1-judge-llm-"))
        .expect("judge llm candidate");
    assert!(llm.caps.iter().any(|cap| cap == "judge:llm"));
    let jev = file
        .candidates
        .iter()
        .find(|candidate| candidate.id.starts_with("p1-jev-"))
        .expect("jev candidate");
    assert!(jev.caps.iter().any(|cap| cap == "judge:jev"));
    assert_eq!(jev.base_url, "http://127.0.0.1:9/v1/systemone");

    let auto = file
        .virtual_models
        .iter()
        .find(|model| model.id == "auto")
        .expect("手写的 auto 组应保留");
    // 编译不建组：判断模型既不进候选池，也不会催生新的厂商组。
    let group_ids: Vec<&str> = file
        .virtual_models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(group_ids, vec!["auto"], "编译不应创建路由组");
    assert!(
        !auto
            .candidate_pool
            .iter()
            .any(|id| id.starts_with("p1-judge-llm-"))
    );
    assert!(
        !auto
            .candidate_pool
            .iter()
            .any(|id| id.starts_with("p1-jev-"))
    );
}

/// 目录勾选后立即重编译：组页候选列表与供应商页选择保持一致。
#[test]
fn select_model_recompiles_candidates_immediately() {
    let h = harness();
    // m2 初始未勾选：编译产物里不应出现。
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("initial compile");
    let text = std::fs::read_to_string(&h.path).expect("read config");
    assert!(!text.contains("id = \"p1-m2-"));

    // 勾选 m2 → select_model 内部自动重编译，无需再点「应用模型选择」。
    provider_service::select_model(
        &h.state,
        "p1",
        CatalogSelectRequest {
            model_id: "m2".to_string(),
            selected: true,
            purpose: None,
            caps: None,
            context_window: None,
            max_output: None,
            price: None,
            tier: None,
            strength: None,
            auto_include_pattern: None,
            clear_deprecated: None,
            hidden: None,
        },
    )
    .expect("select m2");

    let text = std::fs::read_to_string(&h.path).expect("read config");
    assert!(text.contains("id = \"p1-m2-"), "勾选后候选应立即出现");
}

/// 重编译继承候选级设置（上游 model / 输入模态 / 思考档 / 请求头），不被其它模型的勾选抹掉。
#[test]
fn recompile_preserves_candidate_level_settings() {
    let h = harness();
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");
    let text = std::fs::read_to_string(&h.path).expect("read config");
    let mut file = ConfigFile::parse(&text).expect("parse");
    let id = file
        .candidates
        .iter()
        .find(|candidate| candidate.id.starts_with("p1-m1-"))
        .expect("m1 candidate")
        .id
        .clone();
    {
        let candidate = file
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .expect("m1 candidate");
        candidate.model = "custom-upstream".to_string();
        candidate.input = vec![
            tr_contracts::InputModality::Text,
            tr_contracts::InputModality::Image,
        ];
        candidate.thinking_levels = vec!["low".to_string(), "high".to_string()];
    }
    let tuned = toml::to_string_pretty(&file).expect("serialize tuned");
    let revision = tr_control::config_service::disk_revision(&h.state).expect("disk");
    tr_control::config_service::apply(&h.state, &tuned, &revision).expect("apply tuned");

    // 勾选另一个模型触发重编译。
    provider_service::select_model(
        &h.state,
        "p1",
        CatalogSelectRequest {
            model_id: "m2".to_string(),
            selected: true,
            purpose: None,
            caps: None,
            context_window: None,
            max_output: None,
            price: None,
            tier: None,
            strength: None,
            auto_include_pattern: None,
            clear_deprecated: None,
            hidden: None,
        },
    )
    .expect("select m2");

    let text = std::fs::read_to_string(&h.path).expect("read config");
    let file = ConfigFile::parse(&text).expect("parse");
    let candidate = file
        .candidates
        .iter()
        .find(|candidate| candidate.id == id)
        .expect("candidate kept");
    assert_eq!(candidate.model, "custom-upstream");
    assert!(candidate.input.contains(&tr_contracts::InputModality::Text));
    assert!(
        candidate
            .input
            .contains(&tr_contracts::InputModality::Image)
    );
    assert_eq!(
        candidate.thinking_levels,
        vec!["low".to_string(), "high".to_string()]
    );
}

/// 取消勾选仍被组引用的模型：保留为停用候选，配置保持合法（编译不会 422）。
#[test]
fn deselect_referenced_model_keeps_config_valid() {
    let h = harness();
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");
    let text = std::fs::read_to_string(&h.path).expect("read config");
    let compiled_id = ConfigFile::parse(&text)
        .expect("parse")
        .candidates
        .iter()
        .find(|candidate| candidate.id.starts_with("p1-m1-"))
        .expect("m1 compiled")
        .id
        .clone();

    // 手写一个引用 m1 的组（编译不建组，组由用户维护）。
    add_manual_group(&h.path, "group-m1", &compiled_id);

    // 取消勾选 m1：group-m1 的候选池仍引用它 → 必须保留为停用候选，而不是删掉。
    provider_service::select_model(
        &h.state,
        "p1",
        CatalogSelectRequest {
            model_id: "m1".to_string(),
            selected: false,
            purpose: None,
            caps: None,
            context_window: None,
            max_output: None,
            price: None,
            tier: None,
            strength: None,
            auto_include_pattern: None,
            clear_deprecated: None,
            hidden: None,
        },
    )
    .expect("deselect m1");

    let text = std::fs::read_to_string(&h.path).expect("read config");
    let file = ConfigFile::parse(&text).expect("config stays valid");
    let candidate = file
        .candidates
        .iter()
        .find(|candidate| candidate.id == compiled_id)
        .expect("referenced candidate retained");
    assert!(!candidate.enabled, "不再编译但被引用的候选应停用");
    let auto = file
        .virtual_models
        .iter()
        .find(|vm| vm.id == "group-m1")
        .expect("manual group kept");
    assert!(
        auto.candidate_pool.contains(&compiled_id),
        "组池保持引用（停用候选不参与路由）"
    );
}

/// 追加一个用户手写的路由组（编译不建组，测试里只能自己造）。
fn add_manual_group(path: &std::path::Path, group_id: &str, candidate_id: &str) {
    let text = std::fs::read_to_string(path).expect("read config");
    let extra = format!(
        "\n[[virtual_models]]\nprovider = \"token-router\"\nid = \"{group_id}\"\nname = \"{group_id}\"\nprofile = \"default\"\ncandidate_pool = [\"{candidate_id}\"]\n"
    );
    std::fs::write(path, text + &extra).expect("write config");
}

fn compiled_candidate(path: &std::path::Path, prefix: &str) -> tr_agent::config::CandidateFile {
    let text = std::fs::read_to_string(path).expect("read config");
    let file = ConfigFile::parse(&text).expect("parse config");
    file.candidates
        .iter()
        .find(|candidate| candidate.id.starts_with(prefix))
        .expect("compiled candidate")
        .clone()
}

fn change_kind(h: &Harness, kind: ProviderKindDto) {
    provider_service::update_provider(
        &h.state,
        "p1",
        ProviderUpsertRequest {
            id: None,
            template_id: None,
            display_name: None,
            kind: Some(kind),
            base_url: None,
            catalog_url: None,
            enabled: None,
        },
    )
    .expect("update provider kind");
}

#[test]
fn provider_kind_change_migrates_default_protocol_candidates() {
    let h = harness();
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");
    assert_eq!(compiled_candidate(&h.path, "p1-m1-").provider, "openai");

    change_kind(&h, ProviderKindDto::Anthropic);

    assert_eq!(
        compiled_candidate(&h.path, "p1-m1-").provider,
        "anthropic",
        "默认协议应随 provider 类型迁移"
    );
    // 手写候选不属于该 provider，保持原样。
    let text = std::fs::read_to_string(&h.path).expect("read config");
    let file = ConfigFile::parse(&text).expect("parse config");
    let manual = file
        .candidates
        .iter()
        .find(|candidate| candidate.id == "a")
        .expect("manual candidate");
    assert_eq!(manual.provider, "openai");
}

#[test]
fn provider_kind_change_keeps_explicit_protocol_override() {
    let h = harness();
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");

    // 显式把该模型改成 responses（与 provider 默认不同）。
    let text = std::fs::read_to_string(&h.path).expect("read config");
    let mut file = ConfigFile::parse(&text).expect("parse config");
    for candidate in &mut file.candidates {
        if candidate.id.starts_with("p1-m1-") {
            candidate.provider = "responses".to_string();
        }
    }
    std::fs::write(&h.path, toml::to_string_pretty(&file).expect("toml")).expect("write config");

    change_kind(&h, ProviderKindDto::Anthropic);

    assert_eq!(
        compiled_candidate(&h.path, "p1-m1-").provider,
        "responses",
        "显式协议覆盖不应被类型变更抹掉"
    );
}

/// 目录里的推理档位（models.dev 派生）作为候选默认值编译进去。
#[test]
fn compile_uses_catalog_thinking_levels_as_default() {
    let h = harness();
    store::providers::set_model_thinking_levels(
        h.state.pipeline.db(),
        "p1",
        "m1",
        &["low".to_string(), "medium".to_string(), "high".to_string()],
    )
    .expect("set levels");
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");
    let text = std::fs::read_to_string(&h.path).expect("read config");
    let candidate = ConfigFile::parse(&text)
        .expect("parse")
        .candidates
        .into_iter()
        .find(|candidate| candidate.id.starts_with("p1-m1-"))
        .expect("m1 candidate");
    assert_eq!(
        candidate.thinking_levels,
        vec!["low".to_string(), "medium".to_string(), "high".to_string()]
    );
}

/// 回归：编译（供应商变更、目录勾选、供应商类型变更）只写 `[[candidates]]`，
/// 不创建、不改写、不删除任何 `[[virtual_models]]`——路由组必须由用户手动创建。
#[test]
fn compile_never_creates_or_rewrites_virtual_models() {
    let h = harness();
    let groups = |path: &std::path::Path| -> Vec<(String, Vec<String>)> {
        let text = std::fs::read_to_string(path).expect("read config");
        ConfigFile::parse(&text)
            .expect("parse config")
            .virtual_models
            .into_iter()
            .map(|vm| (vm.id, vm.candidate_pool))
            .collect()
    };

    // p1 有启用账号 + 已勾选 m1：编译出候选，但不该冒出 auto-p1 之类的组。
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile");
    assert_eq!(
        groups(&h.path),
        vec![("auto".to_string(), vec!["a".to_string()])],
        "编译不应创建路由组"
    );

    // 目录勾选触发的自动重编译同样不建组。
    provider_service::select_model(
        &h.state,
        "p1",
        CatalogSelectRequest {
            model_id: "m2".to_string(),
            selected: true,
            purpose: None,
            caps: None,
            context_window: None,
            max_output: None,
            price: None,
            tier: None,
            strength: None,
            auto_include_pattern: None,
            clear_deprecated: None,
            hidden: None,
        },
    )
    .expect("select m2");
    assert_eq!(
        groups(&h.path),
        vec![("auto".to_string(), vec!["a".to_string()])],
        "勾选模型后的重编译不应创建路由组"
    );

    // 用户手动建的组也不被编译改写：池子保持手写的值，不会扩成「全厂商」。
    let compiled = compiled_candidate(&h.path, "p1-m2-").id;
    add_manual_group(&h.path, "manual", &compiled);
    provider_service::compile_candidates(
        &h.state,
        CompileRequest {
            expected_revision: None,
        },
    )
    .expect("compile again");
    assert_eq!(
        groups(&h.path),
        vec![
            ("auto".to_string(), vec!["a".to_string()]),
            ("manual".to_string(), vec![compiled]),
        ],
        "编译不应改写手动创建的组"
    );
}
