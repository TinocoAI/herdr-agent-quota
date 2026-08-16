use crate::cache::CacheStore;
use crate::presentation::dashboard_summary;
use crate::providers::agy::run_statusline;
use anyhow::Result;
use std::io::Read;

/// Consume one Agy statusLine JSON payload and update the local snapshot.
///
/// Agy's native `/statusline` command is user-configured, so this hook is
/// intentionally one-shot: it never starts a background process or contacts
/// an external API.
pub fn run_statusline_hook() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let Ok(snapshot) = run_statusline(&input) else {
        return Ok(());
    };
    let cache = CacheStore::from_env()?;
    cache.save(&snapshot)?;
    let now = CacheStore::now_unix();
    println!("Agy | {}", dashboard_summary(&snapshot, now));
    Ok(())
}
