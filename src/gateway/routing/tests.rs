#[cfg(test)]
mod tests {
    use crate::gateway::api::openai::{
        ChatCompletionRequest, ContentPart, FunctionCallPayload, FunctionDefinition, ImageUrl,
        Message, Role, ToolCall, ToolDefinition,
    };
    use crate::config::{ConfigFile, UpstreamEndpoint};

    use crate::gateway::classifier::{ClassifierSettings, ClassifierStore};
    use crate::gateway::config::AppConfig;
    use crate::gateway::experience::{ExperienceSettings, ExperienceStore, RequestOutcome};
    use crate::gateway::multimodal::MultimodalStore;
    use crate::gateway::edge_load::EdgeInferenceTracker;
    use crate::gateway::routing::{
        EffectiveRouting, Profile, RouteDecision, RouteTier, RoutingMode, StepKind, WordFreqStore,
        WorkStrategy, conversation::conversation_key, decide, require_any_upstream,
    };
    use crate::gateway::multimodal::MultimodalStrategy;
    use crate::gateway::session::SessionStore;

    fn test_multimodal_store() -> std::sync::Arc<MultimodalStore> {
        MultimodalStore::new_in_memory()
    }

    fn test_config(edge: bool, cloud: bool) -> AppConfig {
        test_config_with_verify_rate(edge, cloud, 1.0)
    }

    fn test_config_with_verify_rate(edge: bool, cloud: bool, verify_rate: f32) -> AppConfig {
        test_config_with_ctx_max(edge, cloud, verify_rate, ConfigFile::default().gateway.ctx_edge_max_tokens)
    }

    fn test_config_with_ctx_max(
        edge: bool,
        cloud: bool,
        verify_rate: f32,
        ctx_edge_max_tokens: u32,
    ) -> AppConfig {
        let mut file = ConfigFile::default();
        file.gateway.work_verify_sample_rate = verify_rate;
        file.gateway.ctx_edge_max_tokens = ctx_edge_max_tokens;
        if edge {
            file.upstream.edge = Some(UpstreamEndpoint {
                base_url: "http://127.0.0.1:11434/v1".into(),
                api_key: None,
                model: None,
            });
        }
        if cloud {
            file.upstream.cloud = Some(UpstreamEndpoint {
                base_url: "https://api.deepseek.com/v1".into(),
                api_key: Some("test-key".into()),
                model: None,
            });
        }
        AppConfig::from_file(file, std::path::PathBuf::from("/tmp/flowy-test-config.toml"))
            .unwrap()
    }

    fn test_classifier() -> std::sync::Arc<ClassifierStore> {
        ClassifierStore::new_in_memory(ClassifierSettings {
            min_samples: 5,
            ..Default::default()
        })
    }

    fn test_wordfreq() -> std::sync::Arc<WordFreqStore> {
        std::sync::Arc::new(
            WordFreqStore::open_in_memory().expect("test wordfreq store"),
        )
    }

    fn decide_test(
        config: &AppConfig,
        req: &ChatCompletionRequest,
        sessions: &SessionStore,
        experience: Option<&ExperienceStore>,
        multimodal: Option<&MultimodalStore>,
    ) -> crate::gateway::routing::RouteDecision {
        decide(
            config,
            req,
            sessions,
            experience,
            multimodal,
            &EffectiveRouting::passthrough(config),
            None,
            None,
            test_wordfreq().as_ref(),
        )
    }

    fn decide_with_classifier(
        config: &AppConfig,
        req: &ChatCompletionRequest,
        sessions: &SessionStore,
        experience: Option<&ExperienceStore>,
        multimodal: Option<&MultimodalStore>,
        classifier: &ClassifierStore,
    ) -> crate::gateway::routing::RouteDecision {
        decide(
            config,
            req,
            sessions,
            experience,
            multimodal,
            &EffectiveRouting::passthrough(config),
            None,
            Some(classifier),
            test_wordfreq().as_ref(),
        )
    }

    fn heartbeat_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some("You are OpenClaw".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("[OpenClaw heartbeat poll]".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    fn simple_greeting_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "abc".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("你好".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: true,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    fn simple_image_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("描述这张图片".into()),
                content_parts: Some(vec![
                    ContentPart {
                        part_type: "text".into(),
                        text: Some("描述这张图片".into()),
                        image_url: None,
                    },
                    ContentPart {
                        part_type: "image_url".into(),
                        text: None,
                        image_url: Some(ImageUrl {
                            url: "https://example.com/cat.png".into(),
                        }),
                    },
                ]),
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    fn complex_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some("You are a coding agent.".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some(
                        "Refactor the entire authentication module with tests and migration."
                            .into(),
                    ),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn require_any_upstream_rejects_empty() {
        let cfg = test_config(false, false);
        assert!(require_any_upstream(&cfg).is_err());
    }

    fn casual_chat_with_tool_schema_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("讲个短笑话".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "session_status".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: true,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    fn casual_routes_edge(route: RouteTier) -> bool {
        matches!(route, RouteTier::Edge)
    }

    #[test]
    fn casual_chat_with_tool_schema_prefers_edge() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &casual_chat_with_tool_schema_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::DirectChat, "{:?}", decision);
        assert!(
            casual_routes_edge(decision.route),
            "casual should not force cloud: {:?}",
            decision
        );
        assert!(decision.casual_quality_fallback);
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "CASUAL_EDGE_FALLBACK"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn simple_greeting_prefers_edge() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &simple_greeting_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::DirectChat, "{:?}", decision);
        assert!(
            casual_routes_edge(decision.route),
            "casual greeting: {:?} {:?}",
            decision.route,
            decision.reason_codes
        );
    }

    #[test]
    fn heartbeat_prefers_edge() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &heartbeat_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::HeartbeatAck, "{:?}", decision);
        assert!(
            casual_routes_edge(decision.route),
            "heartbeat casual: {:?} {:?}",
            decision.route,
            decision.reason_codes
        );
    }

    #[test]
    fn edge_only_forces_edge_even_for_hard_tasks() {
        let cfg = test_config(true, false);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &complex_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Edge),
            "expected edge-only override, got {:?} {:?}",
            decision.route,
            decision.reason_codes
        );
        assert!(
            decision.reason_codes.iter().any(|c| c == "UPSTREAM_EDGE_ONLY"),
            "{:?}",
            decision.reason_codes
        );
    }

    fn hermes_mid_loop_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some("You are a coding agent.".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("fix the bug".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("I'll run a command.".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Tool,
                    content: Some("ok".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: Some("call_1".into()),
                },
                Message {
                    role: Role::User,
                    content: Some("continue".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: true,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn hermes_mid_loop_not_initial_plan() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &hermes_mid_loop_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_ne!(
            decision.step_kind,
            StepKind::InitialPlan,
            "mid-loop with tool history should not be InitialPlan: {:?}",
            decision
        );
    }

    fn openclaw_system_with_spawn_docs() -> String {
        "Use sessions_spawn for larger work. Do not poll subagents in a loop.".into()
    }

    /// Simulates OpenClaw-sized system + tool schema with a short user turn.
    fn openclaw_large_prompt_greeting_request() -> ChatCompletionRequest {
        let mut req = openclaw_large_prompt_daily_request();
        if let Some(user) = req.messages.iter_mut().find(|m| m.role == Role::User) {
            user.content = Some("你好".into());
        }
        req
    }

    /// Simulates OpenClaw-sized system + tool schema with a short user turn.
    fn openclaw_large_prompt_daily_request() -> ChatCompletionRequest {
        let system_blob = "OpenClaw agent context. ".repeat(8_000);
        let tools = (0..40)
            .map(|i| ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: format!("tool_{i}"),
                    description: Some("parameter docs and examples".repeat(20)),
                    parameters: serde_json::json!({"type":"object","properties":{"x":{"type":"string"}}}),
                },
            })
            .collect();
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some(system_blob),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("你好，今天天气怎么样？".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools,
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn openclaw_large_prompt_greeting_stays_edge() {
        let cfg = test_config_with_ctx_max(true, true, 1.0, 50_000);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &openclaw_large_prompt_greeting_request(),
            &sessions,
            None,
            None,
        );
        assert_eq!(decision.step_kind, StepKind::DirectChat);
        assert!(
            casual_routes_edge(decision.route),
            "single greeting with huge OpenClaw bootstrap: {:?}",
            decision
        );
    }

    #[test]
    fn openclaw_large_prompt_daily_stays_edge_for_casual_user() {
        let cfg = test_config_with_ctx_max(true, true, 1.0, 50_000);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &openclaw_large_prompt_daily_request(),
            &sessions,
            None,
            None,
        );
        assert_eq!(
            decision.step_kind,
            StepKind::DirectChat,
            "short user + huge system/tools should still be casual: {:?}",
            decision
        );
        assert!(
            casual_routes_edge(decision.route),
            "static OpenClaw overhead must not force cloud: {:?}",
            decision
        );
        assert!(
            !decision
                .reason_codes
                .iter()
                .any(|c| c == "GATE_CTX_OVERFLOW"),
            "{:?}",
            decision.reason_codes
        );
    }

    fn openclaw_time_question_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "Minimax-M2.5".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some(openclaw_system_with_spawn_docs()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("你好".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("你好！".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("现在几点了？".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "session_status".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: true,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn openclaw_system_docs_do_not_force_subagent_spawn() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &openclaw_time_question_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_ne!(
            decision.step_kind,
            StepKind::SubagentSpawn,
            "system prompt tool docs must not classify as spawn: {:?}",
            decision
        );
        assert_eq!(
            decision.step_kind,
            StepKind::DirectChat,
            "casual turn with tool schema should be direct chat: {:?}",
            decision
        );
        assert!(
            casual_routes_edge(decision.route),
            "casual time question: {:?}",
            decision
        );
    }

    fn initial_plan_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some("You are a coding agent.".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("Refactor the auth module step by step.".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    fn work_tool_select_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some("agent".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("run tests".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("I'll run the test suite.".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("go ahead".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn initial_plan_forces_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &initial_plan_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::InitialPlan);
        assert!(
            matches!(decision.route, RouteTier::Cloud),
            "{:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "PLAN_INTENT_CLOUD" || c == "INITIAL_PLAN_CLOUD"),
            "{:?}",
            decision.reason_codes
        );
    }

    fn tool_error_streak_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some("fix it".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Tool,
                    content: Some("Error: command failed".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: Some("e1".into()),
                },
                Message {
                    role: Role::Tool,
                    content: Some("失败: exit code 1".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: Some("e2".into()),
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    fn single_tool_error_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some("fix it".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Tool,
                    content: Some("Error: command failed".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: Some("e1".into()),
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    fn successful_tool_result_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some("list files".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Tool,
                    content: Some("file1.txt\nfile2.txt".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: Some("ok1".into()),
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn consecutive_tool_errors_force_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &tool_error_streak_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Cloud),
            "two consecutive tool errors should force cloud: {:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "GATE_TOOL_ERROR_STREAK"),
            "{:?}",
            decision.reason_codes
        );
        assert!(decision.force_cloud_sticky);
    }

    #[test]
    fn single_tool_error_increases_difficulty() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let baseline = decide_test(
            &cfg,
            &successful_tool_result_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        let decision = decide_test(
            &cfg,
            &single_tool_error_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "STEP_RECOVERY_AFTER_FAILURE"),
            "{:?}",
            decision.reason_codes
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "TOOL_ERROR_STREAK_1"),
            "{:?}",
            decision.reason_codes
        );
        assert!(
            decision.difficulty > baseline.difficulty,
            "single tool error should increase difficulty: {} vs {}",
            decision.difficulty,
            baseline.difficulty
        );
    }

    #[test]
    fn single_tool_error_routes_cascade_or_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &single_tool_error_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Cascade | RouteTier::Cloud),
            "single tool error should not route to edge: {:?}",
            decision
        );
        assert!(
            !decision
                .reason_codes
                .iter()
                .any(|c| c == "GATE_TOOL_ERROR_STREAK"),
            "single tool error should not hit hard gate: {:?}",
            decision.reason_codes
        );
    }

    fn tool_loop_request(tool_count: u32) -> ChatCompletionRequest {
        let mut messages = vec![Message {
            role: Role::User,
            content: Some("fix the project".into()),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
        }];
        for i in 0..tool_count {
            messages.push(Message {
                role: Role::Assistant,
                content: None,
                content_parts: None,
                tool_calls: Some(vec![ToolCall {
                    id: format!("call_{i}"),
                    call_type: "function".into(),
                    function: FunctionCallPayload {
                        name: "read".into(),
                        arguments: r#"{"path":"."}"#.into(),
                    },
                }]),
                tool_call_id: None,
            });
            messages.push(Message {
                role: Role::Tool,
                content: Some(format!("output {i}")),
                content_parts: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{i}")),
            });
        }
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages,
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "read".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn four_tool_loop_no_difficulty_bump() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &tool_loop_request(4),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            !decision
                .reason_codes
                .iter()
                .any(|c| c.starts_with("TOOL_LOOP_")),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn five_tool_loop_first_tier() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let baseline = decide_test(
            &cfg,
            &tool_loop_request(4),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        let decision = decide_test(
            &cfg,
            &tool_loop_request(5),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            decision.reason_codes.iter().any(|c| c == "TOOL_LOOP_5"),
            "{:?}",
            decision.reason_codes
        );
        assert!(
            decision.difficulty > baseline.difficulty,
            "5 tools should bump difficulty: {} vs {}",
            decision.difficulty,
            baseline.difficulty
        );
    }

    #[test]
    fn six_tool_loop_stays_first_tier() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let five = decide_test(
            &cfg,
            &tool_loop_request(5),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        let six = decide_test(
            &cfg,
            &tool_loop_request(6),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            six.reason_codes.iter().any(|c| c == "TOOL_LOOP_6"),
            "{:?}",
            six.reason_codes
        );
        assert!(
            (five.difficulty - six.difficulty).abs() < 0.01,
            "5 and 6 tool loops share first tier: {} vs {}",
            five.difficulty,
            six.difficulty
        );
    }

    #[test]
    fn seven_tool_loop_second_tier() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let six = decide_test(
            &cfg,
            &tool_loop_request(6),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        let seven = decide_test(
            &cfg,
            &tool_loop_request(7),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            seven.reason_codes.iter().any(|c| c == "TOOL_LOOP_7"),
            "{:?}",
            seven.reason_codes
        );
        assert!(seven.difficulty > six.difficulty);
    }

    #[test]
    fn eight_tool_loop_cap_tier() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let seven = decide_test(
            &cfg,
            &tool_loop_request(7),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        let eight = decide_test(
            &cfg,
            &tool_loop_request(8),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            eight.reason_codes.iter().any(|c| c == "TOOL_LOOP_8"),
            "{:?}",
            eight.reason_codes
        );
        assert!(eight.difficulty > seven.difficulty);
    }

    #[test]
    fn multi_tool_loop_routes_cascade_or_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &tool_loop_request(8),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Cascade | RouteTier::Cloud),
            "deep tool loop should not route to edge: {:?}",
            decision
        );
    }

    fn plan_intent_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("帮我制定一个重构计划".into()),
                content_parts: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn plan_intent_forces_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &plan_intent_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::InitialPlan);
        assert!(matches!(decision.route, RouteTier::Cloud), "{:?}", decision);
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "PLAN_INTENT_CLOUD" || c == "INITIAL_PLAN_CLOUD"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn work_step_verify_cascade_without_experience() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &work_tool_select_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::ToolSelect);
        assert!(
            matches!(decision.route, RouteTier::Cascade),
            "{:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c.starts_with("WORK_VERIFY_SAMPLE")),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn work_step_skips_verify_at_zero_sample_rate() {
        let cfg = test_config_with_verify_rate(true, true, 0.0);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &work_tool_select_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Edge),
            "{:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c.starts_with("WORK_SAMPLE_SKIP")),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn work_step_uses_cached_edge_when_trusted() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let experience = ExperienceStore::new_in_memory(ExperienceSettings::default());
        for _ in 0..5 {
            experience.record_outcome(
                StepKind::ToolSelect,
                RequestOutcome {
                    edge_ok: true,
                    cascade_fallback: false,
                    upstream_error: false,
                },
            );
        }

        let decision = decide_test(
            &cfg,
            &work_tool_select_request(),
            &sessions,
            Some(experience.as_ref()),
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Edge),
            "{:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "WORK_CACHE_EDGE"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn multimodal_simple_chat_tries_edge() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &simple_image_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::DirectChat);
        assert!(
            matches!(decision.route, RouteTier::Edge),
            "simple multimodal daily chat should prefer edge: {:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "MULTIMODAL_SIMPLE_EDGE"),
            "{:?}",
            decision.reason_codes
        );
        assert!(
            !decision.reason_codes.iter().any(|c| c == "GATE_MULTIMODAL"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn multimodal_edge_only_stays_edge() {
        let cfg = test_config(true, false);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &simple_image_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Edge),
            "edge-only multimodal has no cloud fallback: {:?}",
            decision
        );
    }

    fn complex_image_with_tools_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![Message {
                role: Role::User,
                content: Some("根据截图修复这个 bug".into()),
                content_parts: Some(vec![
                    ContentPart {
                        part_type: "text".into(),
                        text: Some("根据截图修复这个 bug".into()),
                        image_url: None,
                    },
                    ContentPart {
                        part_type: "image_url".into(),
                        text: None,
                        image_url: Some(ImageUrl {
                            url: "https://example.com/bug.png".into(),
                        }),
                    },
                ]),
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "exec".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn multimodal_complex_with_tools_forces_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let store = test_multimodal_store();
        store.record_edge(&cfg, "flowy-auto", true);

        let decision = decide_test(
            &cfg,
            &complex_image_with_tools_request(),
            &sessions,
            None,
            Some(store.as_ref()),
        );
        assert_ne!(decision.step_kind, StepKind::DirectChat);
        assert!(
            matches!(decision.route, RouteTier::Cloud),
            "complex multimodal should force cloud even when edge supports vision: {:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "MULTIMODAL_COMPLEX_CLOUD"),
            "{:?}",
            decision.reason_codes
        );
        assert!(
            !decision
                .reason_codes
                .iter()
                .any(|c| c == "MULTIMODAL_CACHE_EDGE"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn multimodal_uses_cached_cloud_after_probe() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let store = test_multimodal_store();
        store.record_edge(&cfg, "flowy-auto", false);
        store.record_cloud(&cfg, "flowy-auto", true);

        let decision = decide_test(
            &cfg,
            &simple_image_request(),
            &sessions,
            None,
            Some(store.as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Cloud),
            "cached cloud capability should skip probe: {:?}",
            decision
        );
        assert!(
            decision.reason_codes.iter().any(|c| c == "MULTIMODAL_CACHE_CLOUD"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn cloud_only_forces_cloud() {
        let cfg = test_config(false, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &simple_greeting_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert!(
            matches!(decision.route, RouteTier::Cloud),
            "expected cloud-only override, got {:?} {:?}",
            decision.route,
            decision.reason_codes
        );
        assert!(
            decision.reason_codes.iter().any(|c| c == "UPSTREAM_CLOUD_ONLY"),
            "{:?}",
            decision.reason_codes
        );
    }

    fn seed_cloud_sticky(sessions: &SessionStore, conv_key: &str) {
        let decision = RouteDecision {
            route: RouteTier::Cascade,
            profile: Profile::Balanced,
            mode: RoutingMode::Cascade,
            step_kind: StepKind::ToolSelect,
            difficulty: 0.5,
            reason_codes: vec![],
            tokens_in_estimate: 100,
            tokens_out_estimate: 50,
            cloud_input_saved_estimate: 0,
            conversation_key: conv_key.to_string(),
            assistant_failed_recent: false,
            multimodal_strategy: MultimodalStrategy::None,
            work_strategy: WorkStrategy::None,
            force_cloud_sticky: false,
            edge_ok_probability: None,
            classifier_features: None,
            casual_quality_fallback: false,
            lexical_learn: Default::default(),
        };
        sessions.apply_outcome(
            conv_key,
            &decision,
            RequestOutcome {
                edge_ok: false,
                cascade_fallback: true,
                upstream_error: false,
            },
            3600,
            false,
        );
    }

    #[test]
    fn sticky_does_not_block_direct_chat() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let req = simple_greeting_request();
        let key = conversation_key(&req);
        seed_cloud_sticky(&sessions, &key);
        let decision = decide_test(&cfg, &req, &sessions, None, None);
        assert!(
            casual_routes_edge(decision.route),
            "DirectChat should bypass sticky: {:?}",
            decision
        );
        assert!(
            !decision
                .reason_codes
                .iter()
                .any(|c| c == "GATE_STICKY_CLOUD"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn sticky_work_exec_uses_cascade_retry() {
        let cfg = test_config_with_verify_rate(true, true, 0.0);
        let sessions = SessionStore::new_in_memory();
        let req = work_tool_select_request();
        let key = conversation_key(&req);
        seed_cloud_sticky(&sessions, &key);
        let decision = decide_test(&cfg, &req, &sessions, None, None);
        assert_eq!(decision.step_kind, StepKind::ToolSelect);
        assert!(
            matches!(decision.route, RouteTier::Cascade),
            "{:?}",
            decision
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c == "STICKY_CASCADE_RETRY"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn sticky_initial_plan_stays_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let req = initial_plan_request();
        let key = conversation_key(&req);
        seed_cloud_sticky(&sessions, &key);
        let decision = decide_test(&cfg, &req, &sessions, None, None);
        assert_eq!(decision.step_kind, StepKind::InitialPlan);
        assert!(matches!(decision.route, RouteTier::Cloud), "{:?}", decision);
        assert!(
            !decision
                .reason_codes
                .iter()
                .any(|c| c == "STICKY_CASCADE_RETRY"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn edge_ok_clears_cloud_sticky() {
        let sessions = SessionStore::new_in_memory();
        let key = "conv:edge_clear";
        seed_cloud_sticky(&sessions, &key);
        assert!(sessions.cloud_sticky_until(key).is_some());

        let decision = RouteDecision {
            route: RouteTier::Edge,
            profile: Profile::Balanced,
            mode: RoutingMode::Single,
            step_kind: StepKind::DirectChat,
            difficulty: 0.1,
            reason_codes: vec![],
            tokens_in_estimate: 50,
            tokens_out_estimate: 20,
            cloud_input_saved_estimate: 50,
            conversation_key: key.to_string(),
            assistant_failed_recent: false,
            multimodal_strategy: MultimodalStrategy::None,
            work_strategy: WorkStrategy::None,
            force_cloud_sticky: false,
            edge_ok_probability: None,
            classifier_features: None,
            casual_quality_fallback: false,
            lexical_learn: Default::default(),
        };
        sessions.apply_outcome(
            key,
            &decision,
            RequestOutcome::success(&decision, false),
            600,
            false,
        );
        assert!(sessions.cloud_sticky_until(key).is_none());
    }

    #[test]
    fn heartbeat_not_cloud_when_edge_busy() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let tracker = EdgeInferenceTracker::new();
        let _guard = tracker.begin();
        let decision = decide(
            &cfg,
            &heartbeat_request(),
            &sessions,
            None,
            None,
            &EffectiveRouting::passthrough(&cfg),
            Some(tracker.as_ref()),
            None,
            test_wordfreq().as_ref(),
        );
        assert!(
            casual_routes_edge(decision.route),
            "heartbeat casual should stay edge when edge busy: {:?}",
            decision
        );
        assert!(
            !decision.reason_codes.iter().any(|c| c == "GATE_EDGE_BUSY"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn edge_busy_routes_cloud_for_work_when_edge_inference_active() {
        let cfg = test_config_with_verify_rate(true, true, 0.0);
        let sessions = SessionStore::new_in_memory();
        let tracker = EdgeInferenceTracker::new();
        let _guard = tracker.begin();
        let decision = decide(
            &cfg,
            &work_tool_select_request(),
            &sessions,
            None,
            None,
            &EffectiveRouting::passthrough(&cfg),
            Some(tracker.as_ref()),
            None,
            test_wordfreq().as_ref(),
        );
        assert!(
            matches!(decision.route, RouteTier::Cloud),
            "work steps may use cloud while edge is busy: {:?}",
            decision
        );
        assert!(
            decision.reason_codes.iter().any(|c| c == "GATE_EDGE_BUSY"),
            "{:?}",
            decision.reason_codes
        );
    }

    fn openclaw_mid_loop_short_followup_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: Some("agent".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("你好".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("你好！".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some("再讲一个吧".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![ToolDefinition {
                tool_type: "function".into(),
                function: FunctionDefinition {
                    name: "session_status".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                },
            }],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn mid_loop_short_followup_is_direct_chat_not_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &openclaw_mid_loop_short_followup_request(),
            &sessions,
            None,
            None,
        );
        assert_eq!(decision.step_kind, StepKind::DirectChat, "{:?}", decision);
        assert!(
            casual_routes_edge(decision.route),
            "{:?}",
            decision
        );
    }

    #[test]
    fn classifier_emits_bayes_reason_when_enabled() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let classifier = test_classifier();
        for _ in 0..6 {
            classifier.record(
                &crate::gateway::classifier::FeatureVector {
                    keys: vec!["step_kind:direct_chat".into()],
                },
                RequestOutcome {
                    edge_ok: true,
                    cascade_fallback: false,
                    upstream_error: false,
                },
                RouteTier::Edge,
                WorkStrategy::None,
            );
        }
        let decision = decide_with_classifier(
            &cfg,
            &simple_greeting_request(),
            &sessions,
            None,
            None,
            classifier.as_ref(),
        );
        assert!(
            decision
                .reason_codes
                .iter()
                .any(|c| c.starts_with("BAYES_P(")),
            "{:?}",
            decision.reason_codes
        );
        assert!(decision.edge_ok_probability.is_some());
    }

    fn user_reject_request(text: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "flowy-auto".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: Some("你好".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some("这是端侧的回答。".into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: Role::User,
                    content: Some(text.into()),
                    content_parts: None,
                    tool_calls: None,
                    tool_call_id: None,
                },
            ],
            tools: vec![],
            stream: false,
            tool_choice: None,
            max_tokens: None,
            ..Default::default()
        }
    }

    #[test]
    fn user_reject_zh_forces_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &user_reject_request("不对，重新说"),
            &sessions,
            None,
            None,
        );
        assert_eq!(decision.route, RouteTier::Cloud);
        assert!(
            decision.reason_codes.iter().any(|c| c == "GATE_USER_REJECT"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn user_reject_en_forces_cloud() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &user_reject_request("that's wrong, try again"),
            &sessions,
            None,
            None,
        );
        assert_eq!(decision.route, RouteTier::Cloud);
        assert!(decision.reason_codes.iter().any(|c| c == "GATE_USER_REJECT"));
    }

    #[test]
    fn user_reject_without_prior_assistant_does_not_gate() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &simple_greeting_request(),
            &sessions,
            None,
            None,
        );
        assert!(
            !decision.reason_codes.iter().any(|c| c == "GATE_USER_REJECT"),
            "{:?}",
            decision.reason_codes
        );
    }

    #[test]
    fn casual_mid_difficulty_is_edge_not_cascade() {
        let cfg = test_config(true, true);
        let sessions = SessionStore::new_in_memory();
        let decision = decide_test(
            &cfg,
            &casual_chat_with_tool_schema_request(),
            &sessions,
            None,
            Some(test_multimodal_store().as_ref()),
        );
        assert_eq!(decision.step_kind, StepKind::DirectChat);
        assert_eq!(decision.route, RouteTier::Edge);
        assert!(
            !decision.reason_codes.iter().any(|c| c.contains("CASCADE")),
            "{:?}",
            decision.reason_codes
        );
    }
}
