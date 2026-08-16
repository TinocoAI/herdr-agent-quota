use herdr_agent_quota::configure::grok::{apply_at, uninstall_at};
use std::fs;
use tempfile::tempdir;

#[test]
fn grok_turn_hook_refreshes_quota_silently_and_is_reversible() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("herdr-agent-quota.json");

    apply_at(&path).unwrap();
    let installed = fs::read_to_string(&path).unwrap();
    assert!(installed.contains("\"Stop\""));
    assert!(installed.contains("\"StopFailure\""));
    assert!(installed.contains("\"StopCancelled\""));
    assert!(installed.contains("herdr-agent-quota.refresh-grok"));
    assert!(installed.contains(">/dev/null 2>&1"));

    apply_at(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), installed);

    uninstall_at(&path).unwrap();
    assert!(!path.exists());
}

#[test]
fn grok_hook_uninstall_preserves_a_user_owned_file() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("herdr-agent-quota.json");
    fs::write(&path, "{\"hooks\":{\"Stop\":[]}}").unwrap();

    uninstall_at(&path).unwrap();
    assert!(path.exists());
}
