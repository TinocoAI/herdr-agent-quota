use crate::model::{CacheUsage, ContextUsage, ProviderSnapshot, ResetAt};
use crate::providers::ProviderError;
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const TRANSCRIPT_TAIL_BYTES: u64 = 16 * 1024;

pub fn parse_context(
    value: Option<&Value>,
) -> std::result::Result<Option<ContextUsage>, ProviderError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Err(ProviderError::UnsupportedResponse(
            "context_window is not an object".to_string(),
        ));
    };
    let used = object
        .get("used_percentage")
        .or_else(|| object.get("usedPercentage"))
        .and_then(Value::as_f64);
    let remaining = object
        .get("remaining_percentage")
        .or_else(|| object.get("remainingPercentage"))
        .and_then(Value::as_f64);
    let Some(percent) = used.or_else(|| remaining.map(|value| 100.0 - value)) else {
        return Ok(None);
    };
    let cache = parse_cache_usage(object.get("current_usage"));
    ContextUsage::new(percent)
        .map(|context| context.with_cache(cache))
        .map(Some)
        .map_err(|error| ProviderError::UnsupportedResponse(error.to_string()))
}

fn parse_cache_usage(value: Option<&Value>) -> Option<CacheUsage> {
    let object = value?.as_object()?;
    let has_cache_counters = [
        "cache_read_input_tokens",
        "cacheReadInputTokens",
        "cache_creation_input_tokens",
        "cacheCreationInputTokens",
    ]
    .into_iter()
    .any(|name| object.contains_key(name));
    if !has_cache_counters {
        return None;
    }
    let fresh = token_count(object, "input_tokens", "inputTokens");
    let read = token_count(object, "cache_read_input_tokens", "cacheReadInputTokens");
    let creation = token_count(
        object,
        "cache_creation_input_tokens",
        "cacheCreationInputTokens",
    );
    CacheUsage::from_token_counts(fresh, read, creation)
}

fn token_count(object: &serde_json::Map<String, Value>, snake: &str, camel: &str) -> u64 {
    object
        .get(snake)
        .or_else(|| object.get(camel))
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

/// Add a best-effort Claude cache TTL estimate from the local transcript.
///
/// Claude's statusLine contract exposes cache counters but not an entry
/// expiry timestamp. The transcript tail contains the latest assistant usage
/// bucket and timestamp, so this can show an explicitly approximate countdown
/// without scanning the full conversation or making a network request.
pub fn enrich_cache_ttl(snapshot: &mut ProviderSnapshot, statusline: &Value) {
    let Some(context) = snapshot.context.as_mut() else {
        return;
    };
    let Some(cache) = context.cache.as_mut() else {
        return;
    };
    let Some(path) = statusline.get("transcript_path").and_then(Value::as_str) else {
        return;
    };
    let Some(tail) = read_transcript_tail(Path::new(path)) else {
        return;
    };
    let Some((ttl_seconds, last_activity_unix)) = latest_cache_activity(&tail) else {
        return;
    };
    *cache = cache
        .clone()
        .with_ttl_estimate(ttl_seconds, last_activity_unix);
}

fn read_transcript_tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    if start == 0 {
        return Some(text.into_owned());
    }
    text.split_once('\n')
        .map(|(_, complete_lines)| complete_lines.to_string())
}

fn latest_cache_activity(text: &str) -> Option<(u64, u64)> {
    let mut last_activity = None;
    let mut ttl_seconds = None;
    for line in text.lines().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(usage) = entry
            .get("message")
            .and_then(Value::as_object)
            .and_then(|message| message.get("usage"))
            .or_else(|| entry.get("usage"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let read = token_count(usage, "cache_read_input_tokens", "cacheReadInputTokens");
        let creation = token_count(
            usage,
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
        );
        if read == 0 && creation == 0 {
            continue;
        }
        if last_activity.is_none() {
            last_activity = entry.get("timestamp").and_then(parse_timestamp);
        }
        if ttl_seconds.is_none() {
            ttl_seconds = cache_ttl_seconds(usage);
        }
        if let (Some(ttl_seconds), Some(last_activity)) = (ttl_seconds, last_activity) {
            return Some((ttl_seconds, last_activity));
        }
    }
    None
}

fn cache_ttl_seconds(usage: &serde_json::Map<String, Value>) -> Option<u64> {
    let creation = usage.get("cache_creation")?.as_object()?;
    if token_count(
        creation,
        "ephemeral_1h_input_tokens",
        "ephemeral1hInputTokens",
    ) > 0
    {
        return Some(60 * 60);
    }
    if token_count(
        creation,
        "ephemeral_5m_input_tokens",
        "ephemeral5mInputTokens",
    ) > 0
    {
        return Some(5 * 60);
    }
    None
}

fn parse_timestamp(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .and_then(ResetAt::parse)
            .map(ResetAt::unix_seconds)
    })
}
