use crate::cache::CacheStore;
use crate::herdr::{list_agent_panes, publish_tokens};
use crate::model::{MetadataTokens, Provider};
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
    let panes = list_agent_panes().unwrap_or_default();
    let tokens = [(Provider::Agy, MetadataTokens::from_snapshot(&snapshot))];
    let _ = publish_tokens(&panes, &tokens, CacheStore::now_millis());
    println!("Agy | {}", snapshot.summary());
    Ok(())
}
