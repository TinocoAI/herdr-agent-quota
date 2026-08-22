use super::statusline::{settings_path, Adapter};
use crate::cache::CacheStore;
use crate::model::Provider;
use crate::providers::agy::parse_statusline;
use crate::providers::statusline::enrich_cache_session;
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const CONFIG: Adapter = Adapter {
    label: "Agy",
    subcommand: "agy-statusline",
    backup_file: "agy-statusline.original.json",
};

pub fn check() -> Result<()> {
    CONFIG.check(&settings_path(
        "AGY_SETTINGS_FILE",
        ".gemini/antigravity-cli/settings.json",
    )?)
}

pub fn apply() -> Result<()> {
    let cache = CacheStore::from_env()?;
    let executable = std::env::current_exe().context("resolve plugin executable")?;
    apply_at(
        &settings_path("AGY_SETTINGS_FILE", ".gemini/antigravity-cli/settings.json")?,
        cache.root(),
        &executable,
    )
}

pub fn uninstall() -> Result<()> {
    let cache = CacheStore::from_env()?;
    uninstall_at(
        &settings_path("AGY_SETTINGS_FILE", ".gemini/antigravity-cli/settings.json")?,
        cache.root(),
    )
}

pub fn apply_at(settings: &Path, state: &Path, executable: &Path) -> Result<()> {
    CONFIG.apply(settings, state, executable)
}

pub fn uninstall_at(settings: &Path, state: &Path) -> Result<()> {
    CONFIG.uninstall(settings, state)
}

/// Consume one Agy statusLine payload, cache quota silently, then preserve a
/// user-owned statusLine command when one existed before installation.
pub fn run_statusline_hook() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    if let Ok(value) = serde_json::from_slice::<Value>(&input) {
        if let Ok(mut snapshot) = parse_statusline(&value, CacheStore::now_unix()) {
            if let Ok(cache) = CacheStore::from_env() {
                let _ = cache.with_lock(|| {
                    let previous_cache = cache
                        .load(Provider::Agy)
                        .ok()
                        .flatten()
                        .and_then(|snapshot| snapshot.context)
                        .and_then(|context| context.cache);
                    enrich_cache_session(&mut snapshot, &value, previous_cache.as_ref());
                    let session_id = value
                        .get("session_id")
                        .or_else(|| value.get("sessionId"))
                        .and_then(Value::as_str);
                    cache.save_preserving_context_for_session(snapshot, session_id)
                });
            }
        }
    }
    let cache = CacheStore::from_env()?;
    let Some(command) = CONFIG.previous_command(cache.root())? else {
        return Ok(());
    };
    let mut child = Command::new("sh")
        .args(["-c", &command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("run previous Agy statusLine")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&input)?;
    }
    let output = child.wait_with_output()?;
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stdout().flush()?;
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }
    Ok(())
}
