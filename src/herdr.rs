use crate::model::{MetadataTokens, Provider};
use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

const METADATA_TTL_MS: &str = "86400000";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPane {
    pub pane_id: String,
    pub provider: Provider,
    pub topic: String,
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
    for pane in &mut panes {
        if should_read_topic(pane) {
            if let Some(topic) = read_pane_topic(&executable, pane) {
                pane.topic = topic;
            }
        }
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
                    panes.push(AgentPane {
                        pane_id: pane_id.to_string(),
                        provider,
                        topic: map
                            .get("terminal_title_stripped")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
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
        reported += 1;
        let mut command = Command::new(&executable);
        let topic = if pane.topic.trim().is_empty() {
            truncate_topic(&values.quota_topic)
        } else {
            truncate_topic(&pane.topic)
        };
        command
            .args([
                "pane",
                "report-metadata",
                &pane.pane_id,
                "--source",
                "herdr-agent-quota",
            ])
            .args(["--seq", &sequence.to_string()])
            .args(["--ttl-ms", METADATA_TTL_MS])
            .args(["--token", &format!("quota_badge={}", values.quota_badge)])
            .args(["--token", &format!("quota_state={}", values.quota_state)])
            .args(["--token", &format!("quota_icon={}", values.quota_icon)])
            .args([
                "--token",
                &format!("quota_provider={}", values.quota_provider),
            ])
            .args(["--token", &format!("quota_status={}", values.quota_status)])
            .args(["--token", &format!("quota_5h={}", values.quota_5h)])
            .args(["--token", &format!("quota_week={}", values.quota_week)])
            .args([
                "--token",
                &format!("quota_summary={}", values.quota_summary),
            ])
            .args(if topic.trim().is_empty() {
                vec!["--clear-token".to_string(), "quota_topic".to_string()]
            } else {
                vec!["--token".to_string(), format!("quota_topic={topic}")]
            });
        if let Some(error) = &values.quota_error {
            command.args(["--token", &format!("quota_error={error}")]);
        } else {
            command.args(["--clear-token", "quota_error"]);
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

fn should_read_topic(pane: &AgentPane) -> bool {
    let title = pane.topic.trim();
    if title.is_empty() {
        return true;
    }
    if matches!(pane.provider, Provider::Claude | Provider::Agy) {
        return true;
    }
    matches!(
        title.to_ascii_lowercase().as_str(),
        "codex" | "grok" | "agy"
    ) || !title.chars().any(char::is_whitespace)
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
    extract_topic(&text, pane.provider).or_else(|| {
        let fallback = pane.topic.trim();
        (!fallback.is_empty()).then(|| truncate_topic(fallback))
    })
}

fn extract_topic(text: &str, provider: Provider) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let cleaned_line = strip_control_chars(line);
        let line = cleaned_line.trim();
        let candidate = match provider {
            Provider::Claude if line.starts_with('❯') => line.trim_start_matches('❯').trim(),
            Provider::Codex if line.starts_with('›') => line.trim_start_matches('›').trim(),
            _ if line.starts_with('>') => line.trim_start_matches('>').trim(),
            _ => return None,
        };
        if candidate.is_empty() || is_status_line(candidate) {
            return None;
        }
        Some(truncate_topic(candidate))
    })
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
            {"pane_id": "w1:p1", "agent": "codex"},
            {"pane_id": "w1:p2", "agent_session": {"agent": "claude"}},
            {"pane_id": "w1:p3", "agent": "unknown"}
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
                },
                AgentPane {
                    pane_id: "w1:p2".to_string(),
                    provider: Provider::Claude,
                    topic: String::new(),
                },
            ]
        );
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
    fn truncates_topics_without_splitting_utf8() {
        let topic = truncate_topic(&"你好".repeat(50));
        assert!(topic.ends_with('…'));
        assert!(topic.chars().count() <= 78);
    }
}
