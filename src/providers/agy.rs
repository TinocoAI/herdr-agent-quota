use crate::cache::CacheStore;
use crate::model::{Provider, ProviderSnapshot, UsageWindow, WindowKind};
use crate::providers::ProviderError;
use serde_json::Value;

const FIVE_HOUR_KEYS: [&str; 2] = ["gemini-5h", "3p-5h"];
const WEEKLY_KEYS: [&str; 2] = ["gemini-weekly", "3p-weekly"];

/// Parse the quota object emitted by Agy/Antigravity's statusLine JSON.
///
/// Agy reports separate Gemini and third-party (Claude/GPT) pools. The
/// sidebar has one Agy row, so each window is represented conservatively by
/// the lowest remaining percentage across the pools that are present.
pub fn parse_statusline(
    value: &Value,
    fetched_at_unix: u64,
) -> std::result::Result<ProviderSnapshot, ProviderError> {
    let quota = value
        .get("quota")
        .and_then(Value::as_object)
        .ok_or_else(|| ProviderError::UnsupportedResponse("missing quota".to_string()))?;
    let mut windows = Vec::new();
    for (kind, keys) in [
        (WindowKind::FiveHour, &FIVE_HOUR_KEYS[..]),
        (WindowKind::Weekly, &WEEKLY_KEYS[..]),
    ] {
        if let Some(window) = parse_window(quota, kind, keys)? {
            windows.push(window);
        }
    }
    if windows.is_empty() {
        return Err(ProviderError::UnsupportedResponse(
            "quota has no supported windows".to_string(),
        ));
    }
    Ok(ProviderSnapshot::new(
        Provider::Agy,
        windows,
        fetched_at_unix,
    ))
}

fn parse_window(
    quota: &serde_json::Map<String, Value>,
    kind: WindowKind,
    keys: &[&str],
) -> std::result::Result<Option<UsageWindow>, ProviderError> {
    let mut lowest_remaining: Option<f64> = None;
    let mut reset = None;
    for key in keys {
        let Some(bucket) = quota.get(*key) else {
            continue;
        };
        let Some(remaining) = parse_remaining(bucket) else {
            continue;
        };
        if lowest_remaining.is_none_or(|current| remaining < current) {
            lowest_remaining = Some(remaining);
            reset = bucket
                .get("reset_time")
                .or_else(|| bucket.get("resetTime"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
    }
    let Some(remaining) = lowest_remaining else {
        return Ok(None);
    };
    let used = (100.0 - remaining * 100.0).clamp(0.0, 100.0);
    UsageWindow::new(kind, used, reset)
        .map(Some)
        .map_err(|error| ProviderError::UnsupportedResponse(error.to_string()))
}

fn parse_remaining(value: &Value) -> Option<f64> {
    let object = value.as_object()?;
    let raw = object
        .get("remaining_fraction")
        .or_else(|| object.get("remainingFraction"))
        .or_else(|| object.get("remaining_percent"))
        .or_else(|| object.get("remainingPercentage"))
        .and_then(Value::as_f64)?;
    if !raw.is_finite() {
        return None;
    }
    let fraction = if raw <= 1.0 { raw } else { raw / 100.0 };
    Some(fraction.clamp(0.0, 1.0))
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
    fn parses_both_agy_windows_from_official_quota_keys() {
        let value = json!({
            "quota": {
                "gemini-5h": {"remaining_fraction": 0.9969, "reset_time": "short"},
                "gemini-weekly": {"remaining_fraction": 0.8, "reset_time": "week"},
                "3p-5h": {"remaining_fraction": 0.72, "reset_time": "short-low"},
                "3p-weekly": {"remaining_fraction": 0.91}
            }
        });
        let snapshot = parse_statusline(&value, 1).unwrap();
        assert_eq!(snapshot.provider, Provider::Agy);
        assert_eq!(snapshot.summary(), "5h 72% left · week 80% left");
        assert_eq!(
            snapshot
                .window(WindowKind::FiveHour)
                .unwrap()
                .resets_at
                .as_deref(),
            Some("short-low")
        );
    }

    #[test]
    fn ignores_missing_pool_without_marking_the_window_unavailable() {
        let value = json!({
            "quota": {"gemini-weekly": {"remaining_fraction": 0.61}}
        });
        let snapshot = parse_statusline(&value, 1).unwrap();
        assert_eq!(snapshot.summary(), "5h unavailable · week 61% left");
    }

    #[test]
    fn rejects_payload_without_quota_windows() {
        assert!(parse_statusline(&json!({"quota": {}}), 1).is_err());
    }
}
