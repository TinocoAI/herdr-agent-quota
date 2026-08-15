use crate::model::{MetadataTokens, Provider};
use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPane {
    pub pane_id: String,
    pub provider: Provider,
}

pub fn list_agent_panes() -> Result<Vec<AgentPane>> {
    let executable = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let output = Command::new(executable)
        .args(["agent", "list", "--json"])
        .output()
        .context("list Herdr agents")?;
    if !output.status.success() {
        anyhow::bail!("Herdr agent list failed with {}", output.status);
    }
    let value: Value = serde_json::from_slice(&output.stdout).context("parse Herdr agent list")?;
    let mut panes = Vec::new();
    collect_agent_panes(&value, &mut panes);
    panes.sort_by(|left, right| left.pane_id.cmp(&right.pane_id));
    panes.dedup();
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
    for pane in panes {
        let Some((_, values)) = tokens
            .iter()
            .find(|(provider, _)| *provider == pane.provider)
        else {
            continue;
        };
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
            .args(["--token", &format!("quota_badge={}", values.quota_badge)])
            .args(["--token", &format!("quota_state={}", values.quota_state)])
            .args([
                "--token",
                &format!("quota_summary={}", values.quota_summary),
            ]);
        if let Some(error) = &values.quota_error {
            command.args(["--token", &format!("quota_error={error}")]);
        } else {
            command.args(["--clear-token", "quota_error"]);
        }
        let output = command.output().context("report quota metadata to Herdr")?;
        if !output.status.success() {
            anyhow::bail!("Herdr metadata report failed for {}", pane.pane_id);
        }
    }
    Ok(())
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
                    provider: Provider::Codex
                },
                AgentPane {
                    pane_id: "w1:p2".to_string(),
                    provider: Provider::Claude
                },
            ]
        );
    }
}
