use crate::cache::CacheStore;
use crate::model::{CreditUsage, Provider, ProviderSnapshot, ResetAt, UsageWindow, WindowKind};
use crate::providers::codex;
use crate::providers::ProviderError;
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const OPENROUTER_CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";
const ANTHROPIC_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

fn http_agent() -> Result<ureq::Agent> {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10));

    let bundle = std::env::var_os("SSL_CERT_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            let path =
                PathBuf::from(std::env::var_os("HOME")?).join(".hermes/ssl/custom_bundle.pem");
            path.exists().then_some(path)
        });
    if let Some(path) = bundle {
        let file =
            File::open(&path).with_context(|| format!("open TLS CA bundle {}", path.display()))?;
        let mut reader = BufReader::new(file);
        let certs = rustls_pemfile::certs(&mut reader)
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("parse TLS CA bundle {}", path.display()))?;
        let mut roots = rustls::RootCertStore::empty();
        let (added, _) = roots.add_parsable_certificates(certs);
        if added == 0 {
            anyhow::bail!(
                "TLS CA bundle contains no usable certificates: {}",
                path.display()
            );
        }
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        builder = builder.tls_config(Arc::new(tls));
    }
    Ok(builder.build())
}

/// Which backend a Hermes session's active model resolves to. Hermes is a
/// proxy, so the same "Hermes" pane can report Codex 5h/7d windows, native
/// Anthropic 5h/7d windows, or a continuous OpenRouter credit pool depending
/// on what the session actually routes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Codex,
    Anthropic,
    OpenRouter,
}

impl Backend {
    fn from_billing(billing: Option<&str>, model: &str) -> Self {
        match billing {
            Some("openai-codex") => Backend::Codex,
            // A model switch can update `model` before `billing_provider` is
            // persisted for the next turn. Never query Anthropic with a GPT
            // model just because this session used Claude previously.
            Some("anthropic") if model.to_ascii_lowercase().starts_with("gpt-") => Backend::Codex,
            Some("anthropic") => Backend::Anthropic,
            Some(_) => Backend::OpenRouter,
            None => {
                if is_codex_model(model) {
                    Backend::Codex
                } else if is_anthropic_model(model) {
                    Backend::Anthropic
                } else {
                    Backend::OpenRouter
                }
            }
        }
    }

    /// Prefix stamped on the model id so the sidebar can tell which backend
    /// produced it (mirrors the pre-existing `Codex/` convention).
    fn model_prefix(self) -> Option<&'static str> {
        match self {
            Backend::Codex => Some("Codex"),
            Backend::Anthropic => Some("Claude"),
            Backend::OpenRouter => None,
        }
    }
}

/// Fetch one quota snapshot per Hermes session.
///
/// Hermes is a proxy: the active model of each session decides what to report,
/// and different sessions can run different backends at the same time (one pane
/// on Codex windows, another on OpenRouter credits). Returning a snapshot per
/// session lets the sidebar show each pane's own quota instead of forcing every
/// Hermes pane to share one backend.
///
/// The active model is read live from the Hermes state database
/// (`~/.hermes/state.db`, table `sessions` keyed by the agent session id that
/// Herdr exposes), because a `/model` switch inside a live Hermes session is
/// reflected there but never in `config.yaml`.
///
/// - A Codex session (`billing_provider = openai-codex`, `gpt-5.6-luna`,
///   `codex/...`) is reported exactly like a native Codex pane — the 5h/7d
///   windows from the Codex app server, labelled `Codex/<model>`.
/// - Any other session (OpenRouter routes such as `tencent/hy3`) is reported as
///   a continuous OpenRouter credit pool.
///
/// Each returned snapshot is stamped with that session's resolved model and
/// carries no per-session model map, because it is already scoped to one
/// session.
pub fn fetch_for_sessions(
    session_ids: &[String],
    primary_session: Option<&str>,
) -> Result<Vec<(String, ProviderSnapshot)>> {
    let fetched_at_unix = CacheStore::now_unix();
    let session_models = read_session_models(session_ids);

    // When a primary (event/focus) session is known, resolve it first so a
    // /model switch there is reflected immediately. Sessions absent from the
    // state db (e.g. not yet persisted) fall back to the config default.
    let order: Vec<(String, String, Backend)> = if let Some(primary) = primary_session {
        let mut ordered = session_models
            .iter()
            .filter(|(id, _, _)| id == primary)
            .cloned()
            .collect::<Vec<_>>();
        for entry in &session_models {
            if entry.0 != primary {
                ordered.push(entry.clone());
            }
        }
        if ordered.is_empty() {
            let model = current_hermes_model();
            ordered.push((
                primary.to_string(),
                model.clone(),
                Backend::from_billing(None, &model),
            ));
        }
        ordered
    } else if session_models.is_empty() {
        let model = current_hermes_model();
        vec![(
            String::new(),
            model.clone(),
            Backend::from_billing(None, &model),
        )]
    } else {
        session_models.clone()
    };

    let mut out = Vec::new();
    let mut first_error = None;
    for (session_id, model, backend) in order {
        let session_label = if session_id.is_empty() {
            vec![]
        } else {
            vec![(session_id.clone(), model.clone())]
        };
        match build_snapshot(backend, &model, &session_label, fetched_at_unix) {
            Ok(snapshot) => out.push((session_id, snapshot)),
            Err(error) => {
                // Hermes sessions can use different backends. A TLS/provider
                // failure in one old pane must not discard successful quota
                // fetches for every other pane (for example, a Codex session
                // after switching away from Claude). The failed pane keeps its
                // previous cached snapshot while successful panes are updated.
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if out.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    Ok(out)
}

/// Build a Hermes snapshot for one backend decision. `session_label` carries the
/// `(session_id, model)` pairs so the snapshot's token renderer can show the
/// exact model the pane is running.
fn build_snapshot(
    backend: Backend,
    model: &str,
    session_label: &[(String, String)],
    fetched_at_unix: u64,
) -> Result<ProviderSnapshot> {
    let mut snapshot = match backend {
        Backend::Codex => {
            let mut snap = codex::fetch_for_sessions(&[])
                .map_err(|error| anyhow::anyhow!("codex fetch failed: {error}"))?;
            snap.provider = Provider::Hermes;
            // The Hermes provider has no account gate (a shared quota), so drop
            // the Codex account id copied over by the proxy — otherwise the
            // snapshot would look like it belongs to a different account and be
            // treated as unavailable.
            snap.account_id = None;
            snap
        }
        Backend::Anthropic => match anthropic_token() {
            Some(token) if !token.is_empty() => {
                let agent = http_agent()?;
                let response = agent
                    .get(ANTHROPIC_USAGE_URL)
                    .set("Authorization", &format!("Bearer {token}"))
                    .set("Accept", "application/json")
                    .set("anthropic-beta", "oauth-2025-04-20")
                    .call()
                    .map_err(|error| ProviderError::Request(http_error_status(&error)))?;
                let value: Value = response
                    .into_json()
                    .context("decode Anthropic usage response")?;
                let mut snap = parse_anthropic_usage(&value, fetched_at_unix)?;
                snap.provider = Provider::Hermes;
                snap
            }
            _ => ProviderSnapshot::new(Provider::Hermes, vec![], fetched_at_unix),
        },
        Backend::OpenRouter => match openrouter_key() {
            Some(key) if !key.is_empty() => {
                let agent = http_agent()?;
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
        },
    };

    stamp_session_model(&mut snapshot, backend, model, session_label);
    Ok(snapshot)
}

fn stamp_session_model(
    snapshot: &mut ProviderSnapshot,
    backend: Backend,
    model: &str,
    session_label: &[(String, String)],
) {
    // Stamp the resolved model(s) so the sidebar shows exactly what the pane runs.
    if let Some(prefix) = backend.model_prefix() {
        if session_label.is_empty() {
            if !model.is_empty() {
                snapshot.model = Some(format!("{prefix}/{model}"));
            }
        } else {
            for (session_id, model) in session_label {
                snapshot
                    .session_models
                    .insert(session_id.clone(), format!("{prefix}/{model}"));
            }
            snapshot.model = Some(format!("{prefix}/{}", session_label[0].1));
        }
    } else if session_label.is_empty() {
        if !model.is_empty() {
            snapshot.model = Some(model.to_string());
        }
    } else {
        for (session_id, model) in session_label {
            snapshot
                .session_models
                .insert(session_id.clone(), model.clone());
        }
        snapshot.model = Some(session_label[0].1.clone());
    }
}

/// Read `(session_id, model, backend)` for each known Hermes session from the
/// live Hermes state database. Failures (missing db, lock, parse) are silently
/// ignored so the caller can fall back to the config default.
fn read_session_models(session_ids: &[String]) -> Vec<(String, String, Backend)> {
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
    let _ = conn.busy_timeout(Duration::from_millis(500));

    let mut out = Vec::new();
    for session_id in session_ids {
        let Ok((model, billing)): Result<(String, Option<String>), _> = conn.query_row(
            "SELECT model, billing_provider FROM sessions WHERE id = ?1",
            [session_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ) else {
            continue;
        };
        if model.is_empty() {
            continue;
        }
        // `sessions.model` can name a route that never served a main turn: the
        // gateway persists the agent's model after a FAILED turn too, so a
        // provider fallback that errored (e.g. OpenRouter 402) leaves the
        // fallback model in the row while the session keeps running on the
        // primary. Prefer the route that actually billed the latest main
        // conversation call when the recorded model has never billed one.
        let (model, billing) = match last_billed_main_route(&conn, session_id, &model) {
            Some(route) => route,
            None => (model, billing),
        };
        // The `billing_provider` column is the authoritative signal for which
        // backend a session actually routes through — model ids alone are not
        // enough (Codex ids like gpt-5.6-terra don't match a simple pattern,
        // and an Anthropic session may run any claude-* alias).
        let backend = Backend::from_billing(billing.as_deref(), &model);
        out.push((session_id.clone(), model, backend));
    }
    out
}

/// `Some((model, billing_provider))` of the most recent main-conversation call
/// (`task = ''`) when `recorded_model` has no main-conversation usage row of its
/// own; `None` when the recorded model did serve a main call, when there is no
/// usage yet (fresh session or a `/model` switch awaiting its first call), or
/// when the usage table is unavailable.
fn last_billed_main_route(
    conn: &rusqlite::Connection,
    session_id: &str,
    recorded_model: &str,
) -> Option<(String, Option<String>)> {
    let recorded_billed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_model_usage \
             WHERE session_id = ?1 AND task = '' AND model = ?2 AND api_call_count > 0",
            [session_id, recorded_model],
            |row| row.get(0),
        )
        .ok()?;
    if recorded_billed > 0 {
        return None;
    }
    conn.query_row(
        "SELECT model, billing_provider FROM session_model_usage \
         WHERE session_id = ?1 AND task = '' AND api_call_count > 0 AND model <> '' \
         ORDER BY last_seen DESC LIMIT 1",
        [session_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    )
    .ok()
    .map(|(model, billing)| (model, billing.filter(|b| !b.is_empty())))
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

/// Parse the native Anthropic OAuth usage endpoint response
/// (`GET /api/oauth/usage`) into 5h/7d windows, mirroring what the Hermes
/// backend (`agent/account_usage.py::_fetch_anthropic_account_usage`) already
/// does for `/limits`.
fn parse_anthropic_usage(
    value: &Value,
    fetched_at_unix: u64,
) -> std::result::Result<ProviderSnapshot, ProviderError> {
    let mut windows = Vec::new();
    for (key, kind) in [
        ("five_hour", WindowKind::FiveHour),
        ("seven_day", WindowKind::Weekly),
    ] {
        let Some(window) = value.get(key) else {
            continue;
        };
        let Some(utilization) = window.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        let used_percent = if utilization <= 1.0 {
            utilization * 100.0
        } else {
            utilization
        };
        let reset = window.get("resets_at").and_then(|value| {
            value
                .as_u64()
                .map(ResetAt::from_unix_seconds)
                .or_else(|| value.as_str().and_then(ResetAt::parse))
        });
        let window = UsageWindow::new(kind, used_percent.clamp(0.0, 100.0), reset)
            .map_err(|error| ProviderError::UnsupportedResponse(error.to_string()))?;
        windows.push(window);
    }
    Ok(ProviderSnapshot::new(
        Provider::Hermes,
        windows,
        fetched_at_unix,
    ))
}

/// True for model ids that identify a native Anthropic/Claude backend when no
/// `billing_provider` column is available (e.g. config-default fallback).
fn is_anthropic_model(model: &str) -> bool {
    let label = model.to_ascii_lowercase();
    label.starts_with("claude") || label.contains("claude") || label.contains("anthropic")
}

/// Resolve an Anthropic OAuth/API token the same way Hermes does
/// (`agent/anthropic_credentials.py::resolve_anthropic_token`), minus the
/// Claude Code credential file (that refresh flow is out of scope for a
/// read-only sidebar fetch): env vars first, then the Hermes credential pool
/// in `~/.hermes/auth.json`.
fn anthropic_token() -> Option<String> {
    for var in [
        "ANTHROPIC_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_API_KEY",
    ] {
        if let Ok(value) = std::env::var(var) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    let home = std::env::var_os("HOME")?;
    let auth_path = PathBuf::from(home).join(".hermes/auth.json");
    let contents = std::fs::read_to_string(auth_path).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    let entries = value.get("credential_pool")?.get("anthropic")?.as_array()?;
    for entry in entries {
        if entry.get("auth_type").and_then(Value::as_str) != Some("oauth") {
            continue;
        }
        if let Some(token) = entry.get("access_token").and_then(Value::as_str) {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
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

/// Read the active model for `primary_session` (or the config default when no
/// session is known) from the live Hermes state database. Used to detect a
/// `/model` switch during the debounce window so the sidebar updates
/// immediately instead of waiting for the next periodic refresh.
pub fn current_session_model(primary_session: Option<&str>) -> Option<String> {
    let session_ids = primary_session
        .map(|session| vec![session.to_string()])
        .unwrap_or_default();
    let models = read_session_models(&session_ids);
    if let Some((_, model, _)) = models.first() {
        return Some(model.clone());
    }
    let fallback = current_hermes_model();
    if fallback.is_empty() {
        None
    } else {
        Some(fallback)
    }
}

/// Backwards-compatible single-snapshot entry point used by callers that do
/// not have session ids (e.g. manual `refresh --provider hermes`). Uses no
/// primary session, so the backend falls back to the first known session or
/// the config default.
pub fn fetch() -> Result<ProviderSnapshot> {
    let session_ids: Vec<String> = Vec::new();
    let pairs = fetch_for_sessions(&session_ids, None)?;
    Ok(pairs
        .into_iter()
        .next()
        .map(|(_, snapshot)| snapshot)
        .unwrap_or_else(|| ProviderSnapshot::new(Provider::Hermes, vec![], CacheStore::now_unix())))
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

    #[test]
    fn detects_anthropic_models() {
        assert!(is_anthropic_model("claude-sonnet-4-6"));
        assert!(is_anthropic_model("claude-opus-4"));
        assert!(is_anthropic_model("anthropic/claude-3-5"));
        assert!(!is_anthropic_model("gpt-5.6-luna"));
        assert!(!is_anthropic_model("tencent/hy3"));
    }

    #[test]
    fn backend_from_billing_prefers_the_billing_provider_column() {
        assert_eq!(
            Backend::from_billing(Some("anthropic"), "some-alias"),
            Backend::Anthropic
        );
        assert_eq!(
            Backend::from_billing(Some("openai-codex"), "some-alias"),
            Backend::Codex
        );
        assert_eq!(
            Backend::from_billing(Some("anthropic"), "gpt-6-sol"),
            Backend::Codex,
            "a just-switched GPT model must not inherit the previous Anthropic billing route"
        );
        assert_eq!(
            Backend::from_billing(Some("openrouter"), "claude-looks-like-anthropic"),
            Backend::OpenRouter
        );
    }

    #[test]
    fn backend_from_billing_falls_back_to_model_id_when_billing_is_unknown() {
        assert_eq!(
            Backend::from_billing(None, "claude-sonnet-4-6"),
            Backend::Anthropic
        );
        assert_eq!(Backend::from_billing(None, "gpt-5.6-luna"), Backend::Codex);
        assert_eq!(
            Backend::from_billing(None, "tencent/hy3"),
            Backend::OpenRouter
        );
    }

    #[test]
    fn parses_anthropic_oauth_usage_response() {
        let value = json!({
            "five_hour": {"utilization": 0.05, "resets_at": 1_786_795_200},
            "seven_day": {"utilization": 1.0, "resets_at": 1_787_400_000}
        });
        let snapshot = parse_anthropic_usage(&value, 1).unwrap();
        let five_hour = snapshot.window(WindowKind::FiveHour).unwrap();
        assert!((five_hour.used_percent - 5.0).abs() < 1e-9);
        let weekly = snapshot.window(WindowKind::Weekly).unwrap();
        assert!((weekly.used_percent - 100.0).abs() < 1e-9);
    }

    #[test]
    fn stamps_the_claude_prefix_for_an_anthropic_backend_session() {
        let mut snapshot = ProviderSnapshot::new(Provider::Hermes, vec![], 1);
        stamp_session_model(
            &mut snapshot,
            Backend::Anthropic,
            "claude-sonnet-4-6",
            &[("s1".to_string(), "claude-sonnet-4-6".to_string())],
        );
        assert_eq!(
            snapshot.session_models.get("s1").map(String::as_str),
            Some("Claude/claude-sonnet-4-6")
        );
    }

    #[test]
    fn failed_fallback_model_is_replaced_by_last_billed_route() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_model_usage (session_id TEXT, model TEXT, \
             billing_provider TEXT, task TEXT, api_call_count INTEGER, last_seen REAL);
             INSERT INTO session_model_usage VALUES
               ('s1','deepseek/flash','openrouter','title_generation',1,10),
               ('s1','gpt-6.1-sol','openai-codex','',344,20),
               ('s2','claude-opus-5-5','anthropic','',3,5);",
        )
        .unwrap();
        // s1: row says the failed fallback model, which never billed a main call.
        assert_eq!(
            last_billed_main_route(&conn, "s1", "google/gemini-3.7-flash"),
            Some(("gpt-6.1-sol".to_string(), Some("openai-codex".to_string())))
        );
        // Recorded model did bill a main call: keep it.
        assert_eq!(last_billed_main_route(&conn, "s2", "claude-opus-5-5"), None);
        // No usage at all (fresh session / pending /model switch): keep the row.
        assert_eq!(last_billed_main_route(&conn, "s3", "gpt-6.1-sol"), None);
    }

    #[test]
    fn reads_active_model_from_temp_state_db() {
        let dir = tempfile::tempdir().unwrap();
        let hermes = dir.path().join(".hermes");
        std::fs::create_dir_all(&hermes).unwrap();
        let db = hermes.join("state.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, model TEXT, billing_provider TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, model, billing_provider) VALUES ('s1', 'gpt-5.6-terra', 'openai-codex')",
            [],
        )
        .unwrap();
        let previous = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", dir.path()) };
        let model = current_session_model(Some("s1"));
        if let Some(prev) = previous {
            unsafe { std::env::set_var("HOME", prev) };
        } else {
            unsafe { std::env::remove_var("HOME") };
        }
        assert_eq!(model.as_deref(), Some("gpt-5.6-terra"));
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
            "SELECT id FROM sessions WHERE billing_provider = 'openai-codex' LIMIT 1",
            [],
            |row| row.get(0),
        ) else {
            eprintln!("no Codex session found, skipping");
            return;
        };
        let pairs = fetch_for_sessions(&[sid.clone()], None).expect("fetch must succeed");
        let snapshot = pairs
            .into_iter()
            .find(|(id, _)| id == &sid)
            .map(|(_, snapshot)| snapshot)
            .unwrap_or_else(|| panic!("no snapshot for session {sid}"));
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
