use crate::cache::CacheStore;
use crate::model::{CreditUsage, Provider, ProviderSnapshot};
use crate::providers::codex;
use crate::providers::ProviderError;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;

const OPENROUTER_CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";

/// Fetch the quota snapshot for every Hermes pane identified by `session_ids`.
///
/// Hermes is a proxy: the active model of each session decides what to report.
/// The active model is read live from the Hermes state database
/// (`~/.hermes/state.db`, table `sessions` keyed by the agent session id that
/// Herdr exposes), because a `/model` switch inside a live Hermes session is
/// reflected there but never in `config.yaml`.
///
/// - A Codex model (`gpt-5.6-luna`, `codex/...`) is reported exactly like a
///   native Codex pane — the 5h/7d windows from the Codex app server.
/// - Any other model (OpenRouter routes such as `tencent/hy3`) is reported as
///   a continuous OpenRouter credit pool.
///
/// Each session's resolved model is stored under its session id so every
/// Hermes pane shows the model it is actually running, not a shared guess.
pub fn fetch_for_sessions(session_ids: &[String]) -> Result<ProviderSnapshot> {
    let fetched_at_unix = CacheStore::now_unix();
    let session_models = read_session_models(session_ids);

    // Backend decision: if any active session proxies Codex, report Codex
    // windows for the whole provider (Hermes runs one active model family at a
    // time, and the Codex subscription has no OpenRouter credit balance).
    let any_codex = session_models.iter().any(|(_, _, is_codex)| *is_codex);
    let (global_model, is_codex) = if let Some((_, model, is_codex)) = session_models.first() {
        (model.clone(), *is_codex)
    } else {
        let fallback = current_hermes_model();
        (fallback.clone(), is_codex_model(&fallback))
    };
    let is_codex = any_codex || is_codex;

    let mut snapshot = if is_codex {
        let mut snap = codex::fetch_for_sessions(&[])
            .map_err(|error| anyhow::anyhow!("codex fetch failed: {error}"))?;
        snap.provider = Provider::Hermes;
        // The Hermes provider has no account gate (a shared quota), so drop
        // the Codex account id copied over by the proxy — otherwise the
        // snapshot would look like it belongs to a different account and be
        // treated as unavailable.
        snap.account_id = None;
        snap
    } else {
        match openrouter_key() {
            Some(key) if !key.is_empty() => {
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
                parse_credits_response(&value, fetched_at_unix)?
            }
            _ => ProviderSnapshot::new(Provider::Hermes, vec![], fetched_at_unix),
        }
    };

    // Stamp each session with the model it is actually running.
    if is_codex {
        if session_models.is_empty() {
            if !global_model.is_empty() {
                snapshot.model = Some(format!("Codex/{global_model}"));
            }
        } else {
            for (session_id, model, _) in &session_models {
                snapshot
                    .session_models
                    .insert(session_id.clone(), format!("Codex/{model}"));
            }
            snapshot.model = Some(format!("Codex/{}", session_models[0].1));
        }
    } else {
        if session_models.is_empty() {
            if !global_model.is_empty() {
                snapshot.model = Some(global_model);
            }
        } else {
            for (session_id, model, _) in &session_models {
                snapshot
                    .session_models
                    .insert(session_id.clone(), model.clone());
            }
            snapshot.model = Some(session_models[0].1.clone());
        }
    }

    Ok(snapshot)
}

/// Read `(session_id, model, is_codex)` for each known Hermes session from the
/// live Hermes state database. Failures (missing db, lock, parse) are silently
/// ignored so the caller can fall back to the config default.
fn read_session_models(session_ids: &[String]) -> Vec<(String, String, bool)> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec![];
    };
    let db_path = PathBuf::from(home).join(".hermes/state.db");
    if !db_path.exists() {
        return vec![];
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return vec![];
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(500));

    let mut out = Vec::new();
    for session_id in session_ids {
        let Ok(model): Result<String, _> = conn.query_row(
            "SELECT model FROM sessions WHERE id = ?1",
            [session_id.as_str()],
            |row| row.get(0),
        ) else {
            continue;
        };
        if model.is_empty() {
            continue;
        }
        let is_codex = is_codex_model(&model);
        out.push((session_id.clone(), model, is_codex));
    }
    out
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

/// Resolve the active Hermes model from `config.yaml` when no live session is
/// available. `HERMES_INFERENCE_MODEL` is authoritative; otherwise the
/// `model.default` (or first `model:`/`default:`) line is used.
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

/// Backwards-compatible single-snapshot entry point used by callers that do
/// not have session ids (e.g. manual `refresh --provider hermes`).
pub fn fetch() -> Result<ProviderSnapshot> {
    let session_ids: Vec<String> = Vec::new();
    fetch_for_sessions(&session_ids)
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

    /// Live check against the real `~/.hermes/state.db`: a Codex session must
    /// be reported as a Codex-proxied Hermes pane (windows present, model
    /// prefixed with "Codex/"). Ignored by default because it needs a live
    /// Hermes state database.
    #[test]
    #[ignore]
    fn live_proxy_codex_from_state_db() {
        let home = std::env::var_os("HOME").expect("HOME must be set");
        let db = PathBuf::from(home).join(".hermes/state.db");
        let Ok(conn) =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        else {
            eprintln!("state.db unavailable, skipping");
            return;
        };
        let Ok(sid): Result<String, _> = conn.query_row(
            "SELECT id FROM sessions WHERE model = 'gpt-5.6-luna' OR model LIKE 'codex/%' LIMIT 1",
            [],
            |row| row.get(0),
        ) else {
            eprintln!("no Codex session found, skipping");
            return;
        };
        let snapshot = fetch_for_sessions(&[sid.clone()]).expect("fetch must succeed");
        assert!(
            snapshot
                .model
                .as_deref()
                .unwrap_or("")
                .starts_with("Codex/"),
            "expected Codex-proxied model, got {:?}",
            snapshot.model
        );
        assert!(
            !snapshot.windows.is_empty(),
            "Codex proxy must carry 5h/7d windows"
        );
        assert!(
            snapshot
                .session_models
                .get(&sid)
                .map(|m| m.starts_with("Codex/"))
                .unwrap_or(false),
            "session model must be Codex-prefixed"
        );
    }
}
