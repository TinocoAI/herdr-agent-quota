use crate::cache::CacheStore;
use anyhow::{Context, Result};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const HOOK_FILE: &str = "herdr-agent-quota.json";
const LEGACY_REFRESH_ACTION: &str = "herdr-agent-quota.refresh-grok";
const MANAGED_BY: &str = "herdr-agent-quota";

pub fn check() -> Result<()> {
    let path = hook_path()?;
    if is_managed_hook(&path) {
        println!("Grok quota hook is already installed: {}", path.display());
    } else {
        println!(
            "Grok quota hook preview for {}: refresh during active turns and after final replies",
            path.display()
        );
    }
    Ok(())
}

pub fn apply() -> Result<()> {
    let cache = CacheStore::from_env()?;
    let executable = std::env::current_exe().context("resolve plugin executable")?;
    apply_at(&hook_path()?, cache.root(), &executable)
}

pub fn uninstall() -> Result<()> {
    uninstall_at(&hook_path()?)
}

pub fn apply_at(path: &Path, state: &Path, executable: &Path) -> Result<()> {
    let desired = hook_document(state, executable);
    if path.exists() {
        let current = fs::read_to_string(path).context("read Grok quota hook")?;
        if current == desired {
            return Ok(());
        }
        if !current.contains(LEGACY_REFRESH_ACTION) && !current.contains(MANAGED_BY) {
            anyhow::bail!(
                "refusing to replace user-owned Grok hook file {}",
                path.display()
            );
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create Grok hooks directory")?;
    }
    let temporary = path.with_extension("json.herdr-agent-quota.tmp");
    fs::write(&temporary, desired).context("write Grok quota hook")?;
    fs::rename(&temporary, path).context("replace Grok quota hook")?;
    println!(
        "Installed silent Grok active-response refresh hook at {}",
        path.display()
    );
    Ok(())
}

pub fn uninstall_at(path: &Path) -> Result<()> {
    if is_managed_hook(path) {
        fs::remove_file(path).context("remove Grok quota hook")?;
        println!("Removed Grok quota hook from {}", path.display());
    }
    Ok(())
}

fn hook_path() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("GROK_HOME") {
        return Ok(PathBuf::from(home).join("hooks").join(HOOK_FILE));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".grok/hooks").join(HOOK_FILE))
}

fn is_managed_hook(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|contents| {
        contents.contains(LEGACY_REFRESH_ACTION)
            || (contents.contains(MANAGED_BY) && contents.contains("refresh --provider grok"))
    })
}

fn hook_document(state: &Path, executable: &Path) -> String {
    let command = format!(
        "HERDR_PLUGIN_STATE_DIR={} {} refresh --provider grok >/dev/null 2>&1",
        shell_quote(state),
        shell_quote(executable)
    );
    let handler = json!({
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": 3
        }]
    });
    serde_json::to_string_pretty(&json!({
        "hooks": {
            "PostToolUse": [handler.clone()],
            "Stop": [handler.clone()],
            "StopFailure": [handler.clone()],
            "StopCancelled": [handler]
        },
        "managedBy": MANAGED_BY
    }))
    .expect("serialize static Grok hook")
        + "\n"
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}
