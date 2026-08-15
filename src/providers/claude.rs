use crate::cache::CacheStore;
use crate::model::{Provider, ProviderSnapshot, UsageWindow, WindowKind};
use crate::providers::ProviderError;
use serde_json::Value;

pub fn parse_statusline(
    value: &Value,
    fetched_at_unix: u64,
) -> std::result::Result<ProviderSnapshot, ProviderError> {
    let limits = value
        .get("rate_limits")
        .ok_or_else(|| ProviderError::UnsupportedResponse("missing rate_limits".to_string()))?;
    let mut windows = Vec::new();
    if let Some(window) = parse_window(limits.get("five_hour"), WindowKind::FiveHour)? {
        windows.push(window);
    }
    if let Some(window) = parse_window(limits.get("seven_day"), WindowKind::Weekly)? {
        windows.push(window);
    }
    if windows.is_empty() {
        return Err(ProviderError::UnsupportedResponse(
            "rate_limits has no supported windows".to_string(),
        ));
    }
    Ok(ProviderSnapshot::new(
        Provider::Claude,
        windows,
        fetched_at_unix,
    ))
}

fn parse_window(
    value: Option<&Value>,
    kind: WindowKind,
) -> std::result::Result<Option<UsageWindow>, ProviderError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let used = value
        .get("used_percentage")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            ProviderError::UnsupportedResponse(format!("missing {} usage", kind.label()))
        })?;
    let reset = value
        .get("resets_at")
        .and_then(Value::as_str)
        .map(str::to_string);
    UsageWindow::new(kind, used, reset)
        .map(Some)
        .map_err(|error| ProviderError::UnsupportedResponse(error.to_string()))
}

pub fn run_statusline(input: &[u8]) -> std::result::Result<ProviderSnapshot, ProviderError> {
    let value: Value = serde_json::from_slice(input).map_err(|_| {
        ProviderError::UnsupportedResponse("statusLine input is not JSON".to_string())
    })?;
    parse_statusline(&value, CacheStore::now_unix())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_claude_five_hour_and_weekly_limits() {
        let value = json!({
            "rate_limits": {
                "five_hour": {"used_percentage": 58.0, "resets_at": "2026-08-15T12:00:00Z"},
                "seven_day": {"used_percentage": 27.0, "resets_at": "2026-08-22T12:00:00Z"}
            }
        });
        let snapshot = parse_statusline(&value, 1).unwrap();
        assert_eq!(snapshot.summary(), "5h 42% left · wk 73% left");
    }

    #[test]
    fn allows_a_missing_claude_window() {
        let value = json!({"rate_limits": {"five_hour": null}});
        assert!(parse_statusline(&value, 1).is_err());
        let value = json!({
            "rate_limits": {"seven_day": {"used_percentage": 25.0}}
        });
        assert_eq!(parse_statusline(&value, 1).unwrap().windows.len(), 1);
    }

    #[test]
    fn rejects_non_json_statusline_input() {
        assert!(run_statusline(b"not-json").is_err());
    }
}
