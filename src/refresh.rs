use crate::cache::CacheStore;
use crate::herdr::{
    current_agent_provider, list_agent_panes, list_agent_panes_with_topics, publish_tokens,
};
use crate::model::Provider;
use crate::presentation::MetadataTokens;
use crate::providers::{codex, grok};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct ProviderOutcome {
    pub provider: Provider,
    pub available: bool,
    pub from_cache: bool,
    pub error: Option<String>,
}

pub fn run(providers: &[Provider], force: bool, json: bool) -> Result<()> {
    run_internal(providers, force, json, false)
}

fn run_internal(
    providers: &[Provider],
    force: bool,
    json: bool,
    refresh_topics: bool,
) -> Result<()> {
    let cache = CacheStore::from_env()?;
    let outcomes = cache.with_lock(|| refresh_locked(&cache, providers, force))?;
    publish(&cache, providers, false)?;
    if refresh_topics {
        publish(&cache, providers, true)?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&outcomes)?);
    }
    Ok(())
}

pub fn event() -> Result<()> {
    let providers = event_provider()
        .map(|provider| vec![provider])
        .unwrap_or_else(|| Provider::ALL.to_vec());
    run_internal(&providers, false, false, true)
}

pub fn focus() -> Result<()> {
    let Some(provider) = current_agent_provider()? else {
        return Ok(());
    };
    run(&[provider], false, false)
}

fn refresh_locked(
    cache: &CacheStore,
    providers: &[Provider],
    force: bool,
) -> Result<Vec<ProviderOutcome>> {
    let now = CacheStore::now_unix();
    let mut outcomes = Vec::new();
    for provider in providers {
        if !force && cache.should_debounce(*provider, now, 60)? {
            outcomes.push(ProviderOutcome {
                provider: *provider,
                available: cache.load(*provider)?.is_some(),
                from_cache: true,
                error: None,
            });
            continue;
        }
        let fetched = match provider {
            Provider::Codex => codex::fetch(),
            Provider::Grok => grok::fetch(),
            Provider::Claude => match cache.load(Provider::Claude)? {
                Some(snapshot) => Ok(snapshot),
                None => Err(anyhow::anyhow!(
                    "Claude usage is collected by the Claude statusLine hook"
                )),
            },
            Provider::Agy => match cache.load(Provider::Agy)? {
                Some(snapshot) => Ok(snapshot),
                None => Err(anyhow::anyhow!(
                    "Agy usage is collected by the Agy statusLine hook"
                )),
            },
        };
        cache.mark_refresh(*provider, now)?;
        match fetched {
            Ok(snapshot) => {
                if !matches!(provider, Provider::Claude | Provider::Agy) {
                    cache.save(&snapshot)?;
                }
                outcomes.push(ProviderOutcome {
                    provider: *provider,
                    available: true,
                    from_cache: false,
                    error: None,
                });
            }
            Err(error) => outcomes.push(ProviderOutcome {
                provider: *provider,
                available: cache.load(*provider)?.is_some(),
                from_cache: true,
                error: Some(error.to_string()),
            }),
        }
    }
    Ok(outcomes)
}

fn publish(cache: &CacheStore, providers: &[Provider], refresh_topics: bool) -> Result<()> {
    let panes = if refresh_topics {
        list_agent_panes_with_topics(providers)
    } else {
        list_agent_panes()
    }
    .unwrap_or_default();
    let mut tokens = Vec::new();
    let now = CacheStore::now_unix();
    for provider in providers {
        let snapshot = cache.load(*provider)?;
        if let Some(values) = tokens_for_provider(snapshot.as_ref(), now) {
            tokens.push((*provider, values));
        }
    }
    publish_tokens(&panes, &tokens, CacheStore::now_millis())
}

fn event_provider() -> Option<Provider> {
    let input = std::env::var("HERDR_PLUGIN_EVENT_JSON").ok()?;
    let value: Value = serde_json::from_str(&input).ok()?;
    find_agent(&value).and_then(|agent| agent.parse::<Provider>().ok())
}

fn find_agent(value: &Value) -> Option<&str> {
    match value {
        Value::Object(map) => map
            .get("agent")
            .and_then(Value::as_str)
            .or_else(|| map.values().find_map(find_agent)),
        Value::Array(values) => values.iter().find_map(find_agent),
        _ => None,
    }
}

fn tokens_for_provider(
    snapshot: Option<&crate::model::ProviderSnapshot>,
    now_unix: u64,
) -> Option<MetadataTokens> {
    snapshot.map(|snapshot| MetadataTokens::from_snapshot(snapshot, now_unix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderSnapshot, UsageWindow, WindowKind};
    use tempfile::tempdir;

    #[test]
    fn successful_snapshot_is_kept_when_provider_refresh_fails() {
        let directory = tempdir().unwrap();
        let cache = CacheStore::new(directory.path());
        let snapshot = ProviderSnapshot::new(
            Provider::Grok,
            vec![UsageWindow::new(WindowKind::Weekly, 42.5, None).unwrap()],
            1,
        );
        cache.save(&snapshot).unwrap();
        assert_eq!(cache.load(Provider::Grok).unwrap(), Some(snapshot));
    }

    #[test]
    fn missing_snapshot_does_not_overwrite_sidebar_with_unavailable() {
        let values = tokens_for_provider(None, 1);
        assert!(values.is_none());
    }
}
