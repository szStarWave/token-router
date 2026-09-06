//! Shared Flowy claw catalog helpers (chat/image LLM root vs business video root).

/// True when base looks like Flowy claw (`flowyaipc` / `/claw`), not OpenAI / MiniMax official.
pub fn is_flowy_catalog_base(base_url: &str) -> bool {
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    lower.contains("flowyaipc")
        || lower.contains("/claw/")
        || lower.ends_with("/claw")
        || lower.contains("claw/v1")
}

/// `https://server.flowyaipc.cn/claw/v1` → `https://server.flowyaipc.cn/claw`
pub fn flowy_business_base(configured_base: &str) -> String {
    let base = configured_base.trim().trim_end_matches('/');
    if let Some(stripped) = base
        .strip_suffix("/v1")
        .or_else(|| base.strip_suffix("/V1"))
    {
        return stripped.trim_end_matches('/').to_string();
    }
    base.to_string()
}

/// Flowy image/video catalog requires `flowy/…` or `AIPC-…`. Bare names → `AIPC-{name}`.
pub fn normalize_flowy_model(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() {
        return m.to_string();
    }
    let lower = m.to_ascii_lowercase();
    if lower.starts_with("flowy/") || lower.starts_with("aipc-") {
        return m.to_string();
    }
    if m.contains('/') {
        return m.to_string();
    }
    format!("AIPC-{m}")
}

/// When base is Flowy claw, ensure catalog model prefix; otherwise leave unchanged.
pub fn maybe_normalize_flowy_model(base_url: &str, model: &str) -> String {
    if is_flowy_catalog_base(base_url) {
        normalize_flowy_model(model)
    } else {
        model.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_bare_name_gets_aipc_prefix() {
        assert_eq!(
            maybe_normalize_flowy_model(
                "https://server.flowyaipc.cn/claw/v1",
                "Doubao-seedream-5-0-lite"
            ),
            "AIPC-Doubao-seedream-5-0-lite"
        );
        assert_eq!(
            maybe_normalize_flowy_model(
                "https://server.flowyaipc.cn/claw/v1",
                "AIPC-seedream-lite"
            ),
            "AIPC-seedream-lite"
        );
        assert_eq!(
            maybe_normalize_flowy_model(
                "https://server.flowyaipc.cn/claw/v1",
                "flowy/seedream-lite"
            ),
            "flowy/seedream-lite"
        );
        assert_eq!(
            maybe_normalize_flowy_model("https://api.openai.com/v1", "gpt-image-1"),
            "gpt-image-1"
        );
    }
}
