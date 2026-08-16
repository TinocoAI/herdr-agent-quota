use super::statusline::{settings_path, Adapter};
use crate::cache::CacheStore;
use crate::providers::claude::run_statusline;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const CONFIG: Adapter = Adapter {
    label: "Claude",
    subcommand: "claude-statusline",
    backup_file: "claude-statusline.original.json",
};

pub fn check() -> Result<()> {
    CONFIG.check(&settings_path(
        "CLAUDE_SETTINGS_FILE",
        ".claude/settings.json",
    )?)
}

pub fn apply() -> Result<()> {
    let cache = CacheStore::from_env()?;
    let executable = std::env::current_exe().context("resolve plugin executable")?;
    apply_at(
        &settings_path("CLAUDE_SETTINGS_FILE", ".claude/settings.json")?,
        cache.root(),
        &executable,
    )
}

pub fn uninstall() -> Result<()> {
    let cache = CacheStore::from_env()?;
    uninstall_at(
        &settings_path("CLAUDE_SETTINGS_FILE", ".claude/settings.json")?,
        cache.root(),
    )
}

pub fn apply_at(settings: &Path, state: &Path, executable: &Path) -> Result<()> {
    CONFIG.apply(settings, state, executable)
}

pub fn uninstall_at(settings: &Path, state: &Path) -> Result<()> {
    CONFIG.uninstall(settings, state)
}

pub fn run_statusline_hook() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    if let Ok(snapshot) = run_statusline(&input) {
        if let Ok(cache) = CacheStore::from_env() {
            let _ = cache.save(&snapshot);
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
    Ok(())
}
