use crate::model::Provider;
use crate::presentation::MetadataTokens;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Command;

const METADATA_TTL_MS: &str = "86400000";
const METADATA_TOKEN_NAMES: [&str; 16] = [
    "quota_badge",
    "quota_state",
    "quota_icon",
    "quota_provider",
    "quota_status",
    "quota_summary",
    "quota_5h",
    "quota_5h_normal",
    "quota_5h_warning",
    "quota_5h_danger",
    "quota_week",
    "quota_week_normal",
    "quota_week_warning",
    "quota_week_danger",
    "quota_topic",
    "quota_error",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPane {
    pub pane_id: String,
    pub provider: Provider,
    pub topic: String,
    pub tokens: BTreeMap<String, String>,
}

pub fn list_agent_panes() -> Result<Vec<AgentPane>> {
    let executable = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let output = Command::new(&executable)
        .args(["agent", "list"])
        .output()
        .context("list Herdr agents")?;
    if !output.status.success() {
        anyhow::bail!("Herdr agent list failed with {}", output.status);
    }
    let value: Value = serde_json::from_slice(&output.stdout).context("parse Herdr agent list")?;
    let mut panes = Vec::new();
    collect_agent_panes(&value, &mut panes);
    panes.sort_by(|left, right| left.pane_id.cmp(&right.pane_id));
    panes.dedup_by(|left, right| left.pane_id == right.pane_id);
    Ok(panes)
}

pub fn current_agent_provider() -> Result<Option<Provider>> {
    if let Some(agent) = std::env::var_os("HERDR_FOCUSED_PANE_AGENT") {
        if let Ok(provider) = agent.to_string_lossy().parse::<Provider>() {
            return Ok(Some(provider));
        }
    }
    let executable = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let output = Command::new(executable)
        .args(["pane", "current"])
        .output()
        .context("read focused Herdr pane")?;
    if !output.status.success() {
        anyhow::bail!("Herdr pane current failed with {}", output.status);
    }
    let value: Value =
        serde_json::from_slice(&output.stdout).context("parse focused Herdr pane")?;
    Ok(value
        .pointer("/result/pane/agent")
        .and_then(Value::as_str)
        .and_then(|agent| agent.parse::<Provider>().ok()))
}

pub fn list_agent_panes_with_topics() -> Result<Vec<AgentPane>> {
    let executable = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let mut panes = list_agent_panes()?;
    for pane in &mut panes {
        pane.topic = read_pane_topic(&executable, pane).unwrap_or_default();
    }
    Ok(panes)
}

fn collect_agent_panes(value: &Value, panes: &mut Vec<AgentPane>) {
    match value {
        Value::Object(map) => {
            let pane_id = map
                .get("pane_id")
                .or_else(|| map.get("paneId"))
                .and_then(Value::as_str);
            let kind = map
                .get("agent")
                .and_then(Value::as_str)
                .or_else(|| map.get("kind").and_then(Value::as_str))
                .or_else(|| {
                    map.get("agent_session")
                        .and_then(Value::as_object)
                        .and_then(|session| session.get("agent"))
                        .and_then(Value::as_str)
                });
            if let (Some(pane_id), Some(kind)) = (pane_id, kind) {
                if let Ok(provider) = kind.parse::<Provider>() {
                    let tokens: BTreeMap<String, String> = map
                        .get("tokens")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(|tokens| tokens.iter())
                        .filter_map(|(name, value)| {
                            value
                                .as_str()
                                .map(|value| (name.clone(), value.to_string()))
                        })
                        .collect();
                    let topic = tokens.get("quota_topic").cloned().unwrap_or_default();
                    panes.push(AgentPane {
                        pane_id: pane_id.to_string(),
                        provider,
                        // Preserve the last published topic during quota-only
                        // refreshes. Agent events refresh it from pane output.
                        topic,
                        tokens,
                    });
                }
            }
            for child in map.values() {
                collect_agent_panes(child, panes);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_agent_panes(child, panes);
            }
        }
        _ => {}
    }
}

pub fn publish_tokens(
    panes: &[AgentPane],
    tokens: &[(Provider, MetadataTokens)],
    sequence: u64,
) -> Result<()> {
    let executable = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let mut reported = 0usize;
    let mut failed = Vec::new();
    for pane in panes {
        let Some((_, values)) = tokens
            .iter()
            .find(|(provider, _)| *provider == pane.provider)
        else {
            continue;
        };
        let topic = truncate_topic(&pane.topic);
        let desired = desired_tokens(values, &topic);
        if metadata_matches(&pane.tokens, &desired) {
            continue;
        }
        // Herdr versions that repaint metadata can snap a terminal viewport
        // back to the bottom. Never mutate pane metadata while the user is
        // reading scrollback; the next refresh after they return catches up.
        if pane_is_scrolled(&executable, &pane.pane_id) {
            continue;
        }
        reported += 1;
        let mut command = Command::new(&executable);
        command
            .args([
                "pane",
                "report-metadata",
                &pane.pane_id,
                "--source",
                "herdr-agent-quota",
            ])
            .args(["--seq", &sequence.to_string()])
            .args(["--ttl-ms", METADATA_TTL_MS]);
        for name in METADATA_TOKEN_NAMES {
            if let Some(value) = desired.get(name) {
                command.args(["--token", &format!("{name}={value}")]);
            } else {
                command.args(["--clear-token", name]);
            }
        }
        let output = command.output().context("report quota metadata to Herdr")?;
        if !output.status.success() {
            failed.push(pane.pane_id.clone());
        }
    }
    // A pane can exit between `agent list` and this report, and the exit event
    // itself triggers a publish. One stale pane id must not stop the panes
    // that are still alive from being updated.
    if reported > 0 && failed.len() == reported {
        anyhow::bail!(
            "Herdr metadata report failed for every pane: {}",
            failed.join(", ")
        );
    }
    Ok(())
}

fn pane_is_scrolled(executable: &std::ffi::OsStr, pane_id: &str) -> bool {
    let Ok(output) = Command::new(executable)
        .args(["pane", "get", pane_id])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    serde_json::from_slice::<Value>(&output.stdout)
        .ok()
        .and_then(|value| {
            value
                .pointer("/result/pane/scroll/offset_from_bottom")
                .and_then(Value::as_u64)
        })
        .is_some_and(|offset| offset > 0)
}

fn desired_tokens(values: &MetadataTokens, topic: &str) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::from([
        ("quota_badge".to_string(), values.quota_badge.clone()),
        ("quota_state".to_string(), values.quota_state.clone()),
        ("quota_icon".to_string(), values.quota_icon.clone()),
        ("quota_provider".to_string(), values.quota_provider.clone()),
        ("quota_status".to_string(), values.quota_status.clone()),
        ("quota_summary".to_string(), values.quota_summary.clone()),
    ]);
    insert_optional_token(&mut tokens, "quota_5h", &values.quota_5h);
    insert_severity_token(
        &mut tokens,
        "quota_5h",
        &values.quota_5h,
        values.quota_5h_severity,
    );
    insert_optional_token(&mut tokens, "quota_week", &values.quota_week);
    insert_severity_token(
        &mut tokens,
        "quota_week",
        &values.quota_week,
        values.quota_week_severity,
    );
    insert_optional_token(&mut tokens, "quota_topic", topic);
    if let Some(error) = &values.quota_error {
        tokens.insert("quota_error".to_string(), error.clone());
    }
    tokens
}

fn metadata_matches(
    current: &BTreeMap<String, String>,
    desired: &BTreeMap<String, String>,
) -> bool {
    METADATA_TOKEN_NAMES
        .into_iter()
        .all(|name| current.get(name) == desired.get(name))
}

fn insert_severity_token(
    tokens: &mut BTreeMap<String, String>,
    base: &str,
    value: &str,
    severity: Option<crate::model::Severity>,
) {
    if value.trim().is_empty() {
        return;
    }
    let variant = severity_variant(severity);
    tokens.insert(format!("{base}_{variant}"), value.to_string());
}

fn severity_variant(severity: Option<crate::model::Severity>) -> &'static str {
    match severity.unwrap_or(crate::model::Severity::Unknown) {
        crate::model::Severity::Warning => "warning",
        crate::model::Severity::Danger => "danger",
        crate::model::Severity::Normal => "normal",
        crate::model::Severity::Unknown => "warning",
    }
}

fn insert_optional_token(tokens: &mut BTreeMap<String, String>, name: &str, value: &str) {
    if !value.trim().is_empty() {
        tokens.insert(name.to_string(), value.to_string());
    }
}

fn read_pane_topic(executable: &std::ffi::OsStr, pane: &AgentPane) -> Option<String> {
    let output = Command::new(executable)
        .args([
            "pane",
            "read",
            &pane.pane_id,
            "--source",
            "recent",
            "--lines",
            "160",
            "--format",
            "text",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    extract_topic(&text, pane.provider)
}

fn extract_topic(text: &str, provider: Provider) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let cleaned_line = strip_control_chars(line);
        let line = cleaned_line.trim();
        let candidate = prompt_candidate(line, provider)?;
        if candidate.is_empty() || is_status_line(candidate) {
            return None;
        }
        Some(truncate_topic(candidate))
    })
}

fn prompt_candidate(line: &str, provider: Provider) -> Option<&str> {
    let marker = match provider {
        Provider::Claude if line.starts_with('❯') => '❯',
        Provider::Codex if line.starts_with('›') => '›',
        Provider::Grok if line.starts_with('❯') => '❯',
        Provider::Grok | Provider::Agy if line.starts_with('>') => '>',
        _ => return None,
    };
    Some(line.trim_start_matches(marker).trim())
}

fn truncate_topic(value: &str) -> String {
    let characters: Vec<char> = value.chars().collect();
    if characters.len() <= 80 {
        return value.to_string();
    }
    let mut topic: String = characters.into_iter().take(77).collect();
    topic.push('…');
    topic
}

fn strip_control_chars(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .collect()
}

fn is_status_line(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("accept-edits mode:")
        || lower.starts_with("context ")
        || lower.starts_with("session ")
        || lower.starts_with("auto mode")
        || lower.starts_with("shift+tab")
        || matches!(
            lower.as_str(),
            "/clear" | "/compact" | "/help" | "/status" | "/usage" | "/model" | "/config"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovers_canonical_agent_panes_from_nested_json() {
        let value = json!({"result": {"agents": [
            {"pane_id": "w1:p1", "tab_id": "w1:t1", "agent": "codex"},
            {"pane_id": "w1:p2", "tab_id": "w1:t2", "agent_session": {"agent": "claude"}},
            {"pane_id": "w1:p3", "agent": "unknown"}
        ], "tabs": [
            {"tab_id": "w1:t1", "label": "Owner"},
            {"tab_id": "w1:t2", "label": "Executor"}
        ]}});
        let mut panes = Vec::new();
        collect_agent_panes(&value, &mut panes);
        panes.sort_by(|left, right| left.pane_id.cmp(&right.pane_id));
        assert_eq!(
            panes,
            vec![
                AgentPane {
                    pane_id: "w1:p1".to_string(),
                    provider: Provider::Codex,
                    topic: String::new(),
                    tokens: BTreeMap::new(),
                },
                AgentPane {
                    pane_id: "w1:p2".to_string(),
                    provider: Provider::Claude,
                    topic: String::new(),
                    tokens: BTreeMap::new(),
                },
            ]
        );
    }

    #[test]
    fn quota_only_discovery_preserves_the_last_published_topic() {
        let value = json!({"result": {"agents": [{
            "pane_id": "w1:p1",
            "agent": "grok",
            "tokens": {"quota_topic": "latest task"}
        }]}});
        let mut panes = Vec::new();
        collect_agent_panes(&value, &mut panes);
        assert_eq!(panes[0].topic, "latest task");
    }

    #[test]
    fn extracts_latest_agy_prompt_instead_of_status_line() {
        let text = "> older\nHello\n> hi\nHello!\n> Accept-edits mode: file edits auto-approved\n";
        assert_eq!(extract_topic(text, Provider::Agy).as_deref(), Some("hi"));
    }

    #[test]
    fn extracts_latest_claude_prompt_and_skips_clear_command() {
        let text = "❯ /clear\n❯ hi\n⏺ Hi! What can I help with?\n❯\n";
        assert_eq!(extract_topic(text, Provider::Claude).as_deref(), Some("hi"));
    }

    #[test]
    fn ignores_ai_status_title_as_a_topic() {
        let value = json!({
            "pane_id": "w1:p1",
            "agent": "grok",
            "terminal_title_stripped": "Thinking - L7 Learning Reset"
        });
        let mut panes = Vec::new();
        collect_agent_panes(&value, &mut panes);
        assert_eq!(panes[0].topic, "");
    }

    #[test]
    fn publishes_exactly_one_styled_variant_for_each_window() {
        let mut tokens = BTreeMap::new();
        insert_severity_token(
            &mut tokens,
            "quota_week",
            "week 25% reset 2d3h",
            Some(crate::model::Severity::Warning),
        );
        assert_eq!(
            tokens.get("quota_week_warning").map(String::as_str),
            Some("week 25% reset 2d3h")
        );
        assert!(!tokens.contains_key("quota_week_normal"));
        assert!(!tokens.contains_key("quota_week_danger"));
    }

    #[test]
    fn extracts_latest_grok_user_prompt_instead_of_ai_output() {
        let text = "❯ /goal 你在 ti 工作区接手 L7\n先读计划与权威文档，再按七步做 L7 盘点与设计。\n◇ Ran 1 subagent\n计划已读。先冻结坐标并读材料。\n";
        assert_eq!(
            extract_topic(text, Provider::Grok).as_deref(),
            Some("/goal 你在 ti 工作区接手 L7")
        );
    }

    #[test]
    fn truncates_topics_without_splitting_utf8() {
        let topic = truncate_topic(&"你好".repeat(50));
        assert!(topic.ends_with('…'));
        assert!(topic.chars().count() <= 78);
    }
}
