pub mod claude;
pub mod herdr;

use anyhow::Result;

pub fn run(check: bool, apply: bool, uninstall: bool) -> Result<()> {
    if uninstall {
        claude::uninstall()?;
        herdr::uninstall()?;
    } else if apply {
        herdr::apply()?;
        claude::apply()?;
    } else {
        let _ = check;
        herdr::check()?;
        claude::check()?;
    }
    Ok(())
}
