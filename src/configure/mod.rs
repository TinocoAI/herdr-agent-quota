pub mod agy;
pub mod claude;
pub mod grok;
pub mod herdr;

use anyhow::Result;

/// `--check` is also the no-flag default, so it needs no branch of its own.
pub fn run(_check: bool, apply: bool, uninstall: bool) -> Result<()> {
    if uninstall {
        grok::uninstall()?;
        claude::uninstall()?;
        herdr::uninstall()?;
    } else if apply {
        herdr::apply()?;
        claude::apply()?;
        grok::apply()?;
    } else {
        herdr::check()?;
        claude::check()?;
        grok::check()?;
    }
    Ok(())
}
