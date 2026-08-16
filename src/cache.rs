use crate::model::{Provider, ProviderSnapshot};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct CacheStore {
    root: PathBuf,
}

impl CacheStore {
    pub fn from_env() -> Result<Self> {
        let root = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                ProjectDirs::from("dev", "herdr", "herdr-agent-quota")
                    .map(|dirs| dirs.data_local_dir().to_path_buf())
            })
            .context("cannot determine plugin state directory")?;
        Ok(Self { root })
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("create cache directory {}", self.root.display()))
    }

    pub fn load(&self, provider: Provider) -> Result<Option<ProviderSnapshot>> {
        let path = self.snapshot_path(provider);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let snapshot = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse cached {} snapshot", provider.source()))?;
        Ok(Some(snapshot))
    }

    pub fn save(&self, snapshot: &ProviderSnapshot) -> Result<()> {
        self.ensure()?;
        let destination = self.snapshot_path(snapshot.provider);
        let temporary = self.root.join(format!(
            ".{}.{}.tmp",
            snapshot.provider.source(),
            std::process::id()
        ));
        let bytes = serde_json::to_vec_pretty(snapshot).context("serialize quota snapshot")?;
        fs::write(&temporary, bytes).with_context(|| format!("write {}", temporary.display()))?;
        if let Err(error) = fs::rename(&temporary, &destination) {
            // Otherwise a failed rename leaves the scratch file behind, and
            // every later refresh adds another one.
            let _ = fs::remove_file(&temporary);
            return Err(error).with_context(|| {
                format!(
                    "atomically replace {} with {}",
                    destination.display(),
                    temporary.display()
                )
            });
        }
        Ok(())
    }

    pub fn with_lock<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        self.ensure()?;
        let path = self.root.join("refresh.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        file.lock().context("lock refresh state")?;
        let result = operation();
        let unlock_result = file.unlock().context("unlock refresh state");
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    pub fn should_debounce(
        &self,
        provider: Provider,
        now_unix: u64,
        interval_seconds: u64,
    ) -> Result<bool> {
        self.should_debounce_key(provider.source(), now_unix, interval_seconds)
    }

    pub fn should_debounce_key(
        &self,
        key: &str,
        now_unix: u64,
        interval_seconds: u64,
    ) -> Result<bool> {
        let Ok(contents) = fs::read_to_string(self.marker_path(key)) else {
            return Ok(false);
        };
        let Ok(last) = contents.trim().parse::<u64>() else {
            return Ok(false);
        };
        Ok(now_unix.saturating_sub(last) < interval_seconds)
    }

    pub fn mark_refresh(&self, provider: Provider, now_unix: u64) -> Result<()> {
        self.mark_key(provider.source(), now_unix)
    }

    pub fn mark_key(&self, key: &str, now_unix: u64) -> Result<()> {
        self.ensure()?;
        fs::write(self.marker_path(key), now_unix.to_string()).context("write refresh marker")
    }

    pub fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }

    pub fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default()
    }

    fn snapshot_path(&self, provider: Provider) -> PathBuf {
        self.root.join(format!("{}.json", provider.source()))
    }

    fn marker_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.refresh"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Provider, UsageWindow, WindowKind};
    use tempfile::tempdir;

    fn snapshot() -> ProviderSnapshot {
        ProviderSnapshot::new(
            Provider::Grok,
            vec![UsageWindow::new(WindowKind::Weekly, 42.5, None).unwrap()],
            123,
        )
    }

    #[test]
    fn successful_snapshot_round_trips_through_atomic_cache() {
        let directory = tempdir().unwrap();
        let cache = CacheStore::new(directory.path());
        cache.save(&snapshot()).unwrap();
        assert_eq!(cache.load(Provider::Grok).unwrap(), Some(snapshot()));
    }

    #[test]
    fn missing_cache_is_not_an_error() {
        let directory = tempdir().unwrap();
        let cache = CacheStore::new(directory.path());
        assert_eq!(cache.load(Provider::Claude).unwrap(), None);
    }

    #[test]
    fn refresh_marker_debounces_only_within_interval() {
        let directory = tempdir().unwrap();
        let cache = CacheStore::new(directory.path());
        cache.mark_refresh(Provider::Codex, 100).unwrap();
        assert!(cache.should_debounce(Provider::Codex, 120, 60).unwrap());
        assert!(!cache.should_debounce(Provider::Codex, 161, 60).unwrap());
    }
}
