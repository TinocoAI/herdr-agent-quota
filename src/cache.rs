use crate::model::{Provider, ProviderSnapshot};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_WATCH_INTERVAL_SECONDS: u64 = 60;
pub const MIN_WATCH_INTERVAL_SECONDS: u64 = 30;
pub const MAX_WATCH_INTERVAL_SECONDS: u64 = 60 * 60;
const WATCH_INTERVAL_ENV: &str = "HERDR_AGENT_QUOTA_WATCH_INTERVAL_SECONDS";
const WATCH_INTERVAL_FILE: &str = "watch-interval-seconds";

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

    /// Try to claim a named long-running coordination lock.
    ///
    /// Active-turn refreshers are started by two Herdr events at the same
    /// boundary (and there may be several working providers). A non-blocking
    /// OS lock lets the first global watcher own the poll loop while later
    /// starts exit immediately instead of creating duplicate pollers.
    pub fn try_lock_named(&self, name: &str) -> Result<Option<File>> {
        self.ensure()?;
        let path = self.root.join(name);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(error).with_context(|| format!("lock {}", path.display())),
        }
    }

    pub fn stop_turn_watchers(&self) -> Result<()> {
        self.ensure()?;
        fs::write(
            self.root.join("turn-watch.stop"),
            Self::now_millis().to_string(),
        )
        .context("stop active-turn quota watchers")
    }

    pub fn clear_turn_watcher_stop(&self) -> Result<()> {
        let path = self.root.join("turn-watch.stop");
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("clear active-turn watcher stop marker"),
        }
    }

    pub fn turn_watchers_stopped_after(&self, started_millis: u64) -> Result<bool> {
        let path = self.root.join("turn-watch.stop");
        let Ok(value) = fs::read_to_string(path) else {
            return Ok(false);
        };
        Ok(value
            .trim()
            .parse::<u64>()
            .is_ok_and(|stopped| stopped >= started_millis))
    }

    /// Return the configured active-turn polling interval.
    ///
    /// An environment override is useful for one-off runs and installation
    /// scripts; the state file is the persistent user setting. Invalid or
    /// out-of-range values deliberately fall back to the safe default.
    pub fn watch_interval_seconds(&self) -> u64 {
        std::env::var(WATCH_INTERVAL_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .and_then(Self::valid_watch_interval)
            .or_else(|| {
                fs::read_to_string(self.watch_interval_path())
                    .ok()
                    .and_then(|value| value.trim().parse().ok())
                    .and_then(Self::valid_watch_interval)
            })
            .unwrap_or(DEFAULT_WATCH_INTERVAL_SECONDS)
    }

    pub fn set_watch_interval_seconds(&self, seconds: u64) -> Result<()> {
        Self::valid_watch_interval(seconds).with_context(|| {
            format!(
                "watch interval must be between {MIN_WATCH_INTERVAL_SECONDS} and {MAX_WATCH_INTERVAL_SECONDS} seconds"
            )
        })?;
        self.ensure()?;
        fs::write(self.watch_interval_path(), seconds.to_string())
            .context("write active-turn watch interval")
    }

    pub fn clear_watch_interval(&self) -> Result<()> {
        match fs::remove_file(self.watch_interval_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("remove active-turn watch interval"),
        }
    }

    pub fn validate_watch_interval_seconds(seconds: u64) -> Result<u64> {
        Self::valid_watch_interval(seconds).with_context(|| {
            format!(
                "watch interval must be between {MIN_WATCH_INTERVAL_SECONDS} and {MAX_WATCH_INTERVAL_SECONDS} seconds"
            )
        })
    }

    pub fn should_debounce(
        &self,
        provider: Provider,
        now_unix: u64,
        interval_seconds: u64,
    ) -> Result<bool> {
        let Ok(contents) = fs::read_to_string(self.refresh_marker_path(provider)) else {
            return Ok(false);
        };
        let Ok(last) = contents.trim().parse::<u64>() else {
            return Ok(false);
        };
        Ok(now_unix.saturating_sub(last) < interval_seconds)
    }

    pub fn mark_refresh(&self, provider: Provider, now_unix: u64) -> Result<()> {
        self.ensure()?;
        fs::write(self.refresh_marker_path(provider), now_unix.to_string())
            .context("write refresh marker")
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

    fn refresh_marker_path(&self, provider: Provider) -> PathBuf {
        self.root.join(format!("{}.refresh", provider.source()))
    }

    fn watch_interval_path(&self) -> PathBuf {
        self.root.join(WATCH_INTERVAL_FILE)
    }

    fn valid_watch_interval(seconds: u64) -> Option<u64> {
        (MIN_WATCH_INTERVAL_SECONDS..=MAX_WATCH_INTERVAL_SECONDS)
            .contains(&seconds)
            .then_some(seconds)
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

    #[test]
    fn named_turn_lock_is_non_blocking_and_exclusive() {
        let directory = tempdir().unwrap();
        let cache = CacheStore::new(directory.path());
        let first = cache.try_lock_named("codex.turn.lock").unwrap();
        assert!(first.is_some());
        let second = cache.try_lock_named("codex.turn.lock").unwrap();
        assert!(second.is_none());
        drop(first);
        assert!(cache.try_lock_named("codex.turn.lock").unwrap().is_some());
    }

    #[test]
    fn watcher_stop_marker_is_reversible_for_reinstall() {
        let directory = tempdir().unwrap();
        let cache = CacheStore::new(directory.path());
        cache.stop_turn_watchers().unwrap();
        assert!(cache
            .turn_watchers_stopped_after(CacheStore::now_millis().saturating_sub(1))
            .unwrap());
        cache.clear_turn_watcher_stop().unwrap();
        assert!(!cache
            .turn_watchers_stopped_after(CacheStore::now_millis().saturating_sub(1))
            .unwrap());
    }

    #[test]
    fn watch_interval_defaults_and_persists_a_safe_custom_value() {
        let directory = tempdir().unwrap();
        let cache = CacheStore::new(directory.path());
        assert_eq!(
            cache.watch_interval_seconds(),
            DEFAULT_WATCH_INTERVAL_SECONDS
        );
        cache.set_watch_interval_seconds(300).unwrap();
        assert_eq!(cache.watch_interval_seconds(), 300);
        cache.clear_watch_interval().unwrap();
        assert_eq!(
            cache.watch_interval_seconds(),
            DEFAULT_WATCH_INTERVAL_SECONDS
        );
    }

    #[test]
    fn watch_interval_rejects_values_that_are_too_short_or_long() {
        assert!(
            CacheStore::validate_watch_interval_seconds(MIN_WATCH_INTERVAL_SECONDS - 1).is_err()
        );
        assert!(
            CacheStore::validate_watch_interval_seconds(MAX_WATCH_INTERVAL_SECONDS + 1).is_err()
        );
        assert!(CacheStore::validate_watch_interval_seconds(MIN_WATCH_INTERVAL_SECONDS).is_ok());
        assert!(CacheStore::validate_watch_interval_seconds(MAX_WATCH_INTERVAL_SECONDS).is_ok());
    }
}
