use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use dialoguer::{Confirm, Input, Password};

use crate::cli_settings::CliSettings;
use crate::client::GatewayClient;
use crate::config::{
    CLOUD_MODEL_AUTO, UpstreamEndpointPatch, UpstreamEndpointView, UpstreamSetupUpdate,
    UpstreamSetupView, apply_default_upstream, apply_upstream_patch, ensure_initialized,
    load_from_path, save, view_from_config,
};

pub async fn run_setup(
    config_override: &Option<PathBuf>,
    remote: bool,
    json: bool,
    non_interactive: bool,
    patch: UpstreamSetupUpdate,
    reset_defaults: bool,
) -> Result<()> {
    let interactive = should_interact(&patch, json, reset_defaults, non_interactive);
    let patch = if interactive {
        let current = if remote {
            let settings = load_settings(config_override)?;
            let client = GatewayClient::new(
                settings.gateway_url(),
                settings.api_key(),
                settings.admin_token(),
            );
            client
                .setup_get()
                .await
                .context("GET /v1/admin/setup (is gateway running?)")?
        } else {
            let (path, _) = ensure_initialized(config_override.as_deref())?;
            let (file, _) = load_from_path(&path)?;
            view_from_config(&file)
        };
        interactive_patch(&current)?
    } else {
        patch
    };

    let view = if remote {
        run_remote_setup(config_override, reset_defaults, patch, interactive).await?
    } else {
        run_local_setup(config_override, reset_defaults, patch, interactive)?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else if !interactive {
        print_human(&view);
    }
    Ok(())
}

pub fn should_interact(
    patch: &UpstreamSetupUpdate,
    json: bool,
    reset_defaults: bool,
    non_interactive: bool,
) -> bool {
    !non_interactive
        && !json
        && !reset_defaults
        && patch.gateway.as_ref().is_none_or(|g| g.is_empty())
        && patch.edge.is_none()
        && patch.cloud.is_none()
        && patch.agent_id.is_none()
        && IsTerminal::is_terminal(&std::io::stdin())
}

fn interactive_patch(current: &UpstreamSetupView) -> Result<UpstreamSetupUpdate> {
    println!("Flowy 上游配置");
    println!("直接回车保留括号中的当前值；端侧可选。");
    println!();

    let agent_id = prompt_optional_string(
        "Agent ID（留空=全局默认）",
        current.agent_id.as_deref().unwrap_or(""),
    )?;
    let agent_id = if agent_id.is_empty() { None } else { Some(agent_id) };

    let cloud = current.cloud.as_ref();
    let cloud_url = prompt_string(
        "云端 API URL (须含 /v1)",
        cloud.map(|c| c.base_url.as_str()).unwrap_or(""),
    )?;
    let cloud_model = prompt_string(
        "云端模型 (auto=保留客户端 model)",
        cloud
            .and_then(|c| c.model.as_deref())
            .unwrap_or(CLOUD_MODEL_AUTO),
    )?;
    let cloud_key = prompt_api_key("云端", cloud)?;

    let budget_default = current.cloud.as_ref().and_then(|c| c.token_budget).map(|b| b.to_string()).unwrap_or_default();
    let budget_str = prompt_string(
        "云端 Token 预算（5 小时窗口，超预算 Cloud 降级为 Cascade；0=不限）",
        &budget_default,
    )?;
    let token_budget = if budget_str.trim().is_empty() {
        None
    } else {
        match budget_str.trim().parse::<u64>() {
            Ok(v) => Some(Some(v)),
            Err(_) => {
                println!("⚠ 无效数字，已忽略 token 预算");
                None
            }
        }
    };

    let edge_configured = current
        .edge
        .as_ref()
        .is_some_and(|e| e.configured || !e.base_url.is_empty());
    let configure_edge = Confirm::new()
        .with_prompt("配置端侧 (Edge)?")
        .default(edge_configured)
        .interact()?;

    let edge = if configure_edge {
        let edge = current.edge.as_ref();
        Some(UpstreamEndpointPatch {
            base_url: Some(prompt_string(
                "端侧 API URL (须含 /v1)",
                edge.map(|e| e.base_url.as_str()).unwrap_or("http://127.0.0.1:11434/v1"),
            )?),
            model: Some(prompt_optional_string(
                "端侧模型 (留空=不固定)",
                edge.and_then(|e| e.model.as_deref()).unwrap_or(""),
            )?),
            api_key: prompt_api_key("端侧", edge)?,
            clear: false,
            token_budget: None,
        })
    } else if current.edge.is_some() {
        Some(UpstreamEndpointPatch {
            clear: true,
            ..Default::default()
        })
    } else {
        None
    };

    Ok(UpstreamSetupUpdate {
        agent_id,
        cloud: Some(UpstreamEndpointPatch {
            base_url: Some(cloud_url),
            model: Some(cloud_model),
            api_key: cloud_key,
            token_budget,
            clear: false,
        }),
        edge,
        ..Default::default()
    })
}

fn prompt_string(label: &str, default: &str) -> Result<String> {
    Input::new()
        .with_prompt(label)
        .default(default.to_string())
        .allow_empty(true)
        .interact_text()
        .map_err(Into::into)
}

fn prompt_optional_string(label: &str, default: &str) -> Result<String> {
    let value = prompt_string(label, default)?;
    Ok(value.trim().to_string())
}

fn prompt_api_key(
    tier: &str,
    current: Option<&UpstreamEndpointView>,
) -> Result<Option<String>> {
    if current.is_some_and(|c| c.api_key_set) {
        let change = Confirm::new()
            .with_prompt(format!("{tier} API Key 已配置，是否重新输入?"))
            .default(false)
            .interact()?;
        if !change {
            return Ok(None);
        }
    }

    let key = Password::new()
        .with_prompt(format!("{tier} API Key (可选，留空跳过)"))
        .allow_empty_password(true)
        .interact()?;
    let trimmed = key.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn run_local_setup(
    config_override: &Option<PathBuf>,
    reset_defaults: bool,
    patch: UpstreamSetupUpdate,
    interactive: bool,
) -> Result<UpstreamSetupView> {
    let (path, created) = ensure_initialized(config_override.as_deref())?;
    if created && !interactive {
        println!("Created config at {}", path.display());
    }

    let (mut file, config_path) = load_from_path(&path)?;
    let has_patch = patch
        .gateway
        .as_ref()
        .is_some_and(|g| !g.is_empty())
        || patch.edge.is_some()
        || patch.cloud.is_some()
        || patch.agent_id.is_some();
    let settings = CliSettings {
        file: file.clone(),
        config_path: config_path.clone(),
    };

    if reset_defaults {
        apply_default_upstream(&mut file);
    } else if has_patch {
        if !interactive && file.upstream.cloud.is_none() && file.upstream.edge.is_none() {
            apply_default_upstream(&mut file);
        }
        apply_upstream_patch(&mut file, &patch).map_err(|e| anyhow::anyhow!(e))?;
    } else if !interactive {
        apply_default_upstream(&mut file);
    } else {
        bail!("interactive setup produced no patch");
    }

    save(&config_path, &file)?;
    let view = view_from_config(&file);

    if interactive {
        print_interactive_summary(&view, &config_path, &settings.gateway_url());
    } else if reset_defaults {
        println!(
            "Reset upstream setup in {} (cloud model=auto, edge empty)",
            config_path.display()
        );
    } else if created {
        println!(
            "Initialized upstream setup in {} (cloud model=auto, edge empty)",
            config_path.display()
        );
    } else if has_patch {
        println!("Updated upstream setup in {}", config_path.display());
    } else {
        println!("Saved upstream setup to {}", config_path.display());
    }

    if !interactive {
        println!("Configure remotely: http://<gateway.listen>/setup");
    }

    Ok(view)
}

async fn run_remote_setup(
    config_override: &Option<PathBuf>,
    reset_defaults: bool,
    patch: UpstreamSetupUpdate,
    interactive: bool,
) -> Result<UpstreamSetupView> {
    let settings = load_settings(config_override)?;
    let client = GatewayClient::new(
        settings.gateway_url(),
        settings.api_key(),
        settings.admin_token(),
    );

    if reset_defaults {
        client
            .setup_init()
            .await
            .context("POST /v1/admin/setup/init (is gateway running?)")?;
    }

    let view = if patch.edge.is_some() || patch.cloud.is_some() || patch.agent_id.is_some() {
        client
            .setup_update(&patch)
            .await
            .context("POST /v1/admin/setup")?
    } else {
        client
            .setup_get()
            .await
            .context("GET /v1/admin/setup (is gateway running?)")?
    };

    if interactive {
        print_interactive_summary(&view, &settings.config_path, &settings.gateway_url());
    }

    Ok(view)
}

fn print_interactive_summary(view: &UpstreamSetupView, config_path: &PathBuf, gateway_url: &str) {
    println!();
    println!("✓ 配置已保存");
    println!("  文件: {}", config_path.display());
    print_tier_summary("云端", view.cloud.as_ref());
    print_tier_summary("端侧", view.edge.as_ref());
    println!("  Web 配置: {gateway_url}/setup");
}

fn print_tier_summary(name: &str, tier: Option<&UpstreamEndpointView>) {
    match tier.filter(|t| t.configured || !t.base_url.trim().is_empty()) {
        None => println!("  {name}: 未配置"),
        Some(t) => {
            let model = t.model.as_deref().unwrap_or("(未指定)");
            let key = if t.api_key_set { "已设置" } else { "未设置" };
            println!("  {name}: {} (model={model}, key={key})", t.base_url);
        }
    }
}

fn load_settings(config_override: &Option<PathBuf>) -> Result<CliSettings> {
    let path = match config_override {
        Some(p) => p.clone(),
        None => crate::config::config_file()?,
    };
    let (file, config_path) = load_from_path(&path)?;
    Ok(CliSettings { file, config_path })
}

fn print_human(view: &UpstreamSetupView) {
    let g = &view.gateway;
    println!("Gateway setup");
    if let Some(ref agent_id) = view.agent_id {
        println!("  agent_id:                 {}", agent_id);
    }
    println!("  route:                    {}", g.route);
    println!("  routing_mode:             {}", g.routing_mode);
    println!("  default_profile:          {}", g.default_profile);
    println!("  ctx_edge_max_tokens:      {}", g.ctx_edge_max_tokens);
    println!("  experience_enabled:       {}", g.experience_enabled);
    println!("  work_verify_sample_rate:  {:.2}", g.work_verify_sample_rate);
    println!("  adaptive_routing_enabled: {}", g.adaptive_routing_enabled);
    if let Some(budget) = view.cloud.as_ref().and_then(|c| c.token_budget) {
        println!("  cloud_token_budget:       {}", budget);
    }
    print_tier("edge", view.edge.as_ref());
    print_tier("cloud", view.cloud.as_ref());
}

fn print_tier(name: &str, tier: Option<&UpstreamEndpointView>) {
    match tier {
        None => println!("  {name}: (not configured)"),
        Some(t) => {
            println!("  {name}:");
            println!("    configured: {}", t.configured);
            let url = if t.base_url.is_empty() {
                "(empty)".to_string()
            } else {
                t.base_url.clone()
            };
            println!("    base_url:   {url}");
            println!(
                "    model:      {}",
                t.model.as_deref().unwrap_or("(default)")
            );
            println!(
                "    api_key:    {}",
                if t.api_key_set { "(set)" } else { "(not set)" }
            );
        }
    }
}

pub fn patch_from_cli(
    edge_url: Option<String>,
    edge_key: Option<String>,
    edge_model: Option<String>,
    cloud_url: Option<String>,
    cloud_key: Option<String>,
    cloud_model: Option<String>,
    clear_edge: bool,
) -> UpstreamSetupUpdate {
    UpstreamSetupUpdate {
        edge: if clear_edge || edge_url.is_some() || edge_key.is_some() || edge_model.is_some() {
            Some(UpstreamEndpointPatch {
                base_url: edge_url,
                api_key: edge_key,
                model: edge_model,
                clear: clear_edge,
                ..Default::default()
            })
        } else {
            None
        },
        cloud: if cloud_url.is_some() || cloud_key.is_some() || cloud_model.is_some() {
            Some(UpstreamEndpointPatch {
                base_url: cloud_url,
                api_key: cloud_key,
                model: cloud_model,
                clear: false,
                ..Default::default()
            })
        } else {
            None
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interact_when_tty_flags_absent() {
        assert!(should_interact(
            &UpstreamSetupUpdate::default(),
            false,
            false,
            false,
        ) == IsTerminal::is_terminal(&std::io::stdin()));
    }

    #[test]
    fn no_interact_with_cli_patch() {
        assert!(!should_interact(
            &UpstreamSetupUpdate {
                cloud: Some(UpstreamEndpointPatch {
                    base_url: Some("https://x/v1".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            false,
            false,
            false,
        ));
    }

    #[test]
    fn no_interact_when_non_interactive_flag() {
        assert!(!should_interact(
            &UpstreamSetupUpdate::default(),
            false,
            false,
            true,
        ));
    }
}
