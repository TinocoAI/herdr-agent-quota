use crate::cache::CacheStore;
use crate::model::{Provider, ProviderSnapshot, UsageWindow, WindowKind};
use crate::providers::ProviderError;
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokCredentials {
    pub key: String,
    pub user_id: Option<String>,
}

pub fn fetch() -> Result<ProviderSnapshot> {
    let path = auth_path().context("resolve Grok auth path")?;
    let credentials = read_credentials(&path).map_err(anyhow::Error::from)?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        .build();
    let mut request = agent
        .get(BILLING_URL)
        .set("Authorization", &format!("Bearer {}", credentials.key))
        .set("X-XAI-Token-Auth", "xai-grok-cli")
        .set("Accept", "application/json");
    if let Some(user_id) = &credentials.user_id {
        request = request.set("x-userid", user_id);
    }
    let response = request
        .call()
        .map_err(|error| ProviderError::Request(http_error_status(&error)))
        .map_err(anyhow::Error::from)?;
    let value: Value = response
        .into_json()
        .context("decode Grok billing response")?;
    parse_billing_response(&value, CacheStore::now_unix()).map_err(anyhow::Error::from)
}

pub fn auth_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GROK_AUTH_FILE") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let home = PathBuf::from(home);
    let grok_home = std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".grok"));
    Ok(grok_home.join("auth.json"))
}

pub fn read_credentials(path: &Path) -> std::result::Result<GrokCredentials, ProviderError> {
    let bytes = fs::read(path).map_err(|_| ProviderError::MissingCredentials)?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ProviderError::Unavailable("Grok auth file is not valid JSON".to_string()))?;
    find_credentials(&value).ok_or(ProviderError::MissingCredentials)
}

fn find_credentials(value: &Value) -> Option<GrokCredentials> {
    match value {
        Value::Object(map) => {
            if let Some(key) = map.get("key").and_then(Value::as_str) {
                if !key.trim().is_empty() {
                    let user_id = map
                        .get("user_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    return Some(GrokCredentials {
                        key: key.to_string(),
                        user_id,
                    });
                }
            }
            map.values().find_map(find_credentials)
        }
        Value::Array(values) => values.iter().find_map(find_credentials),
        _ => None,
    }
}

pub fn parse_billing_response(
    value: &Value,
    fetched_at_unix: u64,
) -> std::result::Result<ProviderSnapshot, ProviderError> {
    let config = value
        .get("config")
        .ok_or_else(|| ProviderError::UnsupportedResponse("missing config".to_string()))?;
    let usage = config
        .get("creditUsagePercent")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            ProviderError::UnsupportedResponse("missing config.creditUsagePercent".to_string())
        })?;
    let period = config
        .get("currentPeriod")
        .ok_or_else(|| ProviderError::UnsupportedResponse("missing currentPeriod".to_string()))?;
    let period_type = period
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !period_type.contains("WEEKLY") {
        return Err(ProviderError::UnsupportedResponse(format!(
            "current period is not weekly: {period_type}"
        )));
    }
    let reset = period
        .get("end")
        .and_then(Value::as_str)
        .map(str::to_string);
    let window = UsageWindow::new(WindowKind::Weekly, usage, reset)
        .map_err(|error| ProviderError::UnsupportedResponse(error.to_string()))?;
    Ok(ProviderSnapshot::new(
        Provider::Grok,
        vec![window],
        fetched_at_unix,
    ))
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
    use tempfile::tempdir;

    #[test]
    fn parses_grok_weekly_credit_pool_as_remaining_percentage() {
        let value = json!({
            "config": {
                "creditUsagePercent": 42.5,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "end": "2026-08-22T00:00:00Z"
                }
            }
        });
        let snapshot = parse_billing_response(&value, 1).unwrap();
        assert_eq!(snapshot.provider, Provider::Grok);
        assert_eq!(
            snapshot
                .window(WindowKind::Weekly)
                .unwrap()
                .remaining_percent,
            57.5
        );
    }

    #[test]
    fn rejects_monthly_period_instead_of_calling_it_weekly() {
        let value = json!({
            "config": {
                "creditUsagePercent": 42.5,
                "currentPeriod": {"type": "USAGE_PERIOD_TYPE_MONTHLY"}
            }
        });
        assert!(parse_billing_response(&value, 1).is_err());
    }

    #[test]
    fn reads_only_login_key_from_nested_auth_shape() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("auth.json");
        fs::write(
            &path,
            r#"{"auth.x.ai":{"oidc":{"key":"login-token","refresh_token":"do-not-read","user_id":"u1"}}}"#,
        )
        .unwrap();
        assert_eq!(
            read_credentials(&path).unwrap(),
            GrokCredentials {
                key: "login-token".to_string(),
                user_id: Some("u1".to_string())
            }
        );
    }

    #[test]
    fn missing_auth_is_unavailable() {
        let directory = tempdir().unwrap();
        assert_eq!(
            read_credentials(&directory.path().join("missing.json"))
                .unwrap_err()
                .to_string(),
            "provider credentials are unavailable"
        );
    }
}
