use crate::cache::CacheStore;
use crate::model::{CreditUsage, Provider, ProviderSnapshot};
use crate::providers::codex;
use crate::providers::ProviderError;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

const OPENROUTER_CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";

/// Fetch the quota for the Hermes agent pane.
///
/// Hermes is a proxy: the active model decides which backend to report.
/// - A Codex model (`gpt-5.6-luna`, `codex/...`) is reported exactly like a
///   native Codex pane would be — the 5h/7d windows from the Codex app server.
/// - Any other model (OpenRouter routes such as `tencent/hy3`, or a bare
///   `openrouter/...` id) is reported as a continuous OpenRouter credit pool.
///
/// The active model is read from `HERMES_INFERENCE_MODEL` when present
/// (set inside the Hermes process on a `/model` change) and otherwise from
/// `~/.hermes/config.yaml`.
pub fn fetch() -> Result<ProviderSnapshot> {
    let fetched_at_unix = CacheStore::now_unix();
    let model = current_hermes_model();

    if is_codex_model(&model) {
        let mut snapshot = codex::fetch_for_sessions(&[])
            .map_err(|error| anyhow::anyhow!("codex fetch failed: {error}"))?;
        snapshot.provider = Provider::Hermes;
        // The Hermes provider has no account gate (a shared quota), so drop
        // the Codex account id copied over by the proxy — otherwise the
        // snapshot would look like it belongs to a different account and be
        // treated as unavailable.
        snapshot.account_id = None;
        let model_label = if model.is_empty() {
            "codex".to_string()
        } else {
            model.clone()
        };
        // Identical label to a native Codex pane: "Codex/<model>".
        snapshot.model = Some(format!("Codex/{model_label}"));
        return Ok(snapshot);
    }

    // OpenRouter (or any non-Codex backend): continuous credit balance.
    let key = match openrouter_key() {
        Some(key) if !key.is_empty() => key,
        _ => {
            let mut snapshot = ProviderSnapshot::new(Provider::Hermes, vec![], fetched_at_unix);
            if !model.is_empty() {
                snapshot.model = Some(model);
            }
            return Ok(snapshot);
        }
    };

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_secs(10))
        .timeout_write(std::time::Duration::from_secs(10))
        .build();

    let response = agent
        .get(OPENROUTER_CREDITS_URL)
        .set("Authorization", &format!("Bearer {key}"))
        .set("Accept", "application/json")
        .call()
        .map_err(|error| ProviderError::Request(http_error_status(&error)))?;

    let value: Value = response
        .into_json()
        .context("decode OpenRouter credits response")?;

    let mut snapshot = parse_credits_response(&value, fetched_at_unix)?;
    if !model.is_empty() {
        snapshot.model = Some(model);
    }
    Ok(snapshot)
}

fn parse_credits_response(
    value: &Value,
    fetched_at_unix: u64,
) -> std::result::Result<ProviderSnapshot, ProviderError> {
    let data = value
        .get("data")
        .ok_or_else(|| ProviderError::UnsupportedResponse("missing data".to_string()))?;
    let total = data
        .get("total_credits")
        .and_then(Value::as_f64)
        .ok_or_else(|| ProviderError::UnsupportedResponse("missing total_credits".to_string()))?;
    let used = data
        .get("total_usage")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let credits = CreditUsage::new(total, used);

    let mut snapshot = ProviderSnapshot::new(Provider::Hermes, vec![], fetched_at_unix);
    snapshot.credits = Some(credits);
    Ok(snapshot)
}

/// Resolve the active Hermes model. `HERMES_INFERENCE_MODEL` is authoritative
/// (it reflects a live `/model` switch inside the Hermes process); otherwise
/// the `model.default` (or first `model:`/`default:`) line of the config is
/// used. Returns an empty string when nothing can be determined.
fn current_hermes_model() -> String {
    if let Ok(model) = std::env::var("HERMES_INFERENCE_MODEL") {
        let model = model.trim().to_string();
        if !model.is_empty() {
            return model;
        }
    }
    let Some(home) = std::env::var_os("HOME") else {
        return String::new();
    };
    let config_path = PathBuf::from(home).join(".hermes/config.yaml");
    let Ok(text) = std::fs::read_to_string(&config_path) else {
        return String::new();
    };
    let mut in_model = false;
    for line in text.lines() {
        let stripped = line.trim();
        if stripped.starts_with("model:") {
            in_model = true;
            continue;
        }
        if !in_model {
            continue;
        }
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with("default:") {
            return stripped
                .split_once(':')
                .map(|(_, value)| value.trim().to_string())
                .unwrap_or_default();
        }
        // Left the `model:` block once we hit another top-level key.
        if !line.starts_with(' ') && !line.starts_with('\t') && stripped.contains(':') {
            in_model = false;
        }
    }
    String::new()
}

/// True for ChatGPT/Codex subscription models, which carry no OpenRouter
/// credit balance and must be reported with the Codex windows instead.
fn is_codex_model(model: &str) -> bool {
    let label = model.to_ascii_lowercase();
    label == "gpt-5.6-luna" || label.starts_with("codex") || label.contains("codex")
}

/// Resolve the OpenRouter API key the same way the Hermes runtime does: the
/// process environment first, then `~/.hermes/.env`.
fn openrouter_key() -> Option<String> {
    if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
        if !key.trim().is_empty() {
            return Some(key.trim().to_string());
        }
    }
    let home = std::env::var_os("HOME")?;
    let env_path = PathBuf::from(home).join(".hermes/.env");
    let contents = std::fs::read_to_string(env_path).ok()?;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.starts_with("OPENROUTER_API_KEY") {
            continue;
        }
        if let Some((_, value)) = line.split_once('=') {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn http_error_status(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(error) => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_openrouter_credit_pool() {
        let value = json!({
            "data": {
                "total_credits": 10.0,
                "total_usage": 1.5
            }
        });
        let snapshot = parse_credits_response(&value, 1).unwrap();
        assert_eq!(snapshot.provider, Provider::Hermes);
        let credits = snapshot.credits.unwrap();
        assert!((credits.remaining_percent - 85.0).abs() < 1e-9);
    }

    #[test]
    fn missing_total_is_unsupported() {
        let value = json!({ "data": { "total_usage": 1.5 } });
        assert!(parse_credits_response(&value, 1).is_err());
    }

    #[test]
    fn detects_codex_models() {
        assert!(is_codex_model("gpt-5.6-luna"));
        assert!(is_codex_model("codex/gpt-5"));
        assert!(is_codex_model("some-codex-thing"));
        assert!(!is_codex_model("tencent/hy3"));
        assert!(!is_codex_model("openrouter/anthropic/claude"));
    }
}
