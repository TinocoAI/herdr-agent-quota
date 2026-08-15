use crate::cache::CacheStore;
use crate::herdr::{list_agent_panes, publish_tokens};
use crate::model::{MetadataTokens, Provider};
use crate::providers::claude::run_statusline;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BACKUP_FILE: &str = "claude-statusline.original.json";

pub fn check() -> Result<()> {
    let path = settings_path()?;
    let settings = read_settings(&path)?;
    if is_installed(settings.get("statusLine")) {
        println!(
            "Claude statusLine wrapper is already installed: {}",
            path.display()
        );
    } else {
        println!(
            "Claude statusLine preview for {}: install a reversible quota wrapper",
            path.display()
        );
    }
    Ok(())
}

pub fn apply() -> Result<()> {
    let path = settings_path()?;
    let mut settings = read_settings(&path)?;
    if is_installed(settings.get("statusLine")) {
        return Ok(());
    }
    if !can_chain_statusline(settings.get("statusLine")) {
        anyhow::bail!(
            "existing Claude statusLine has no safely chainable command; refusing to replace it"
        );
    }
    let cache = CacheStore::from_env()?;
    cache.ensure()?;
    let backup = cache.root().join(BACKUP_FILE);
    if !backup.exists() {
        let original = settings.get("statusLine").cloned().unwrap_or(Value::Null);
        fs::write(&backup, serde_json::to_vec_pretty(&original)?)
            .context("write Claude statusLine backup")?;
    }
    let executable = std::env::current_exe().context("resolve plugin executable")?;
    let wrapper_command = format!(
        "HERDR_PLUGIN_STATE_DIR={} {} claude-statusline",
        shell_quote(cache.root()),
        shell_quote(&executable)
    );
    let status_line = settings
        .get_mut("statusLine")
        .and_then(Value::as_object_mut)
        .map(|object| {
            object.insert("type".to_string(), Value::String("command".to_string()));
            object.insert(
                "command".to_string(),
                Value::String(wrapper_command.clone()),
            );
            Value::Object(object.clone())
        })
        .unwrap_or_else(|| {
            json!({
                "type": "command",
                "command": wrapper_command,
            })
        });
    settings["statusLine"] = status_line;
    write_settings(&path, &settings)
}

pub fn uninstall() -> Result<()> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read_settings(&path)?;
    if !is_installed(settings.get("statusLine")) {
        return Ok(());
    }
    let cache = CacheStore::from_env()?;
    let backup = cache.root().join(BACKUP_FILE);
    let original: Value = if backup.exists() {
        serde_json::from_slice(&fs::read(&backup)?)?
    } else {
        Value::Null
    };
    if original.is_null() {
        settings
            .as_object_mut()
            .context("Claude settings must be an object")?
            .remove("statusLine");
    } else {
        settings["statusLine"] = original;
    }
    write_settings(&path, &settings)?;
    if backup.exists() {
        fs::remove_file(backup).context("remove Claude statusLine backup")?;
    }
    Ok(())
}

pub fn run_statusline_hook() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    if let Ok(snapshot) = run_statusline(&input) {
        if let Ok(cache) = CacheStore::from_env() {
            if cache.save(&snapshot).is_ok() {
                let panes = list_agent_panes().unwrap_or_default();
                let tokens = [(Provider::Claude, MetadataTokens::from_snapshot(&snapshot))];
                let _ = publish_tokens(&panes, &tokens, CacheStore::now_unix());
            }
        }
    }
    if let Some(command) = previous_statusline_command()? {
        let mut child = Command::new("sh")
            .args(["-c", &command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("run previous Claude statusLine")?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&input)?;
        }
        let output = child.wait_with_output()?;
        std::io::stdout().write_all(&output.stdout)?;
        std::io::stdout().flush()?;
        if !output.status.success() {
            std::process::exit(output.status.code().unwrap_or(1));
        }
    }
    Ok(())
}

fn can_chain_statusline(status_line: Option<&Value>) -> bool {
    match status_line {
        None | Some(Value::Null) | Some(Value::String(_)) => true,
        Some(Value::Object(map)) => map
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| !command.trim().is_empty()),
        Some(_) => false,
    }
}

fn previous_statusline_command() -> Result<Option<String>> {
    let cache = CacheStore::from_env()?;
    let backup = cache.root().join(BACKUP_FILE);
    if !backup.exists() {
        return Ok(None);
    }
    let value: Value = serde_json::from_slice(&fs::read(backup)?)?;
    Ok(match value {
        Value::String(command) => Some(command),
        Value::Object(map) => map
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    })
}

fn settings_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CLAUDE_SETTINGS_FILE") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".claude/settings.json"))
}

fn read_settings(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_slice(&fs::read(path).context("read Claude settings")?)
        .context("parse Claude settings")?;
    if !value.is_object() {
        anyhow::bail!("Claude settings must be a JSON object")
    }
    Ok(value)
}

fn write_settings(path: &Path, settings: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create Claude settings directory")?;
    }
    let temporary = path.with_extension("json.herdr-agent-quota.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(temporary, path).context("replace Claude settings")
}

fn is_installed(status_line: Option<&Value>) -> bool {
    status_line
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
        .map(|command| {
            command.contains("herdr-agent-quota") && command.contains("claude-statusline")
        })
        .unwrap_or(false)
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detects_wrapper_without_touching_other_statusline_shapes() {
        assert!(is_installed(Some(&json!({
            "type": "command",
            "command": "/tmp/herdr-agent-quota claude-statusline"
        }))));
        assert!(!is_installed(Some(&json!("echo original"))));
    }

    #[test]
    fn settings_round_trip_preserves_unrelated_keys() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, r#"{"theme":"dark","statusLine":"echo old"}"#).unwrap();
        let mut settings = read_settings(&path).unwrap();
        settings["statusLine"] =
            json!({"type":"command","command":"/tmp/herdr-agent-quota claude-statusline"});
        write_settings(&path, &settings).unwrap();
        let saved = read_settings(&path).unwrap();
        assert_eq!(saved["theme"], "dark");
        assert!(is_installed(saved.get("statusLine")));
    }

    #[test]
    fn wrapper_replacement_keeps_existing_statusline_options() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(
            &path,
            r#"{"statusLine":{"type":"command","command":"echo old","refreshInterval":5}}"#,
        )
        .unwrap();
        let mut settings = read_settings(&path).unwrap();
        let object = settings["statusLine"].as_object_mut().unwrap();
        object.insert(
            "command".to_string(),
            Value::String("/tmp/herdr-agent-quota claude-statusline".to_string()),
        );
        write_settings(&path, &settings).unwrap();
        let saved = read_settings(&path).unwrap();
        assert_eq!(saved["statusLine"]["refreshInterval"], 5);
    }

    #[test]
    fn refuses_statusline_shapes_that_cannot_be_chained() {
        assert!(can_chain_statusline(Some(&json!("echo old"))));
        assert!(can_chain_statusline(Some(&json!({
            "type": "command",
            "command": "echo old"
        }))));
        assert!(!can_chain_statusline(Some(&json!({"type": "prompt"}))));
        assert!(!can_chain_statusline(Some(&json!(["echo old"]))));
    }
}
