pub mod agy;
pub mod claude;
pub mod grok;
pub mod herdr;
mod statusline;

use anyhow::{Context, Result};

/// `--check` is also the no-flag default, so it needs no branch of its own.
pub fn run(_check: bool, apply: bool, uninstall: bool) -> Result<()> {
    if apply || uninstall {
        std::env::var_os("HERDR_PLUGIN_STATE_DIR").context(
            "configuration writes must run through Herdr so every collector uses the same cache; invoke herdr-agent-quota.configure or herdr-agent-quota.uninstall",
        )?;
    }
    if uninstall {
        grok::uninstall()?;
        agy::uninstall()?;
        claude::uninstall()?;
        herdr::uninstall()?;
    } else if apply {
        herdr::apply()?;
        claude::apply()?;
        agy::apply()?;
        grok::apply()?;
    } else {
        herdr::check()?;
        claude::check()?;
        agy::check()?;
        grok::check()?;
    }
    Ok(())
}
