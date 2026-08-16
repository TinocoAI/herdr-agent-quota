use herdr_agent_quota::configure::herdr::{add_quota_row, remove_quota_row};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn install_herdr_stub(state: &Path, agent_list: &str) -> (PathBuf, PathBuf) {
    let log = state.join("herdr.log");
    let executable = state.join("herdr");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nif [ \"$1 $2\" = \"agent list\" ]; then\n  printf '%s\\n' '{}'\nelif [ \"$1 $2\" = \"pane read\" ]; then\n  printf '%s\\n' \"$*\" >> '{}'\nelif [ \"$1 $2\" = \"pane report-metadata\" ]; then\n  printf '%s\\n' \"$*\" >> '{}'\nfi\n",
            agent_list,
            log.display(),
            log.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();
    (executable, log)
}

fn run_claude_collector(state: &Path, herdr: &Path, input: &[u8]) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-agent-quota"))
        .arg("claude-statusline")
        .env("HERDR_PLUGIN_STATE_DIR", state)
        .env("HERDR_BIN_PATH", herdr)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
}

fn run_claude_refresh(state: &Path, herdr: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_herdr-agent-quota"))
        .args(["refresh", "--provider", "claude", "--force"])
        .env("HERDR_PLUGIN_STATE_DIR", state)
        .env("HERDR_BIN_PATH", herdr)
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn sidebar_configuration_is_idempotent_and_reversible() {
    let original = "[ui.sidebar.agents]\nrows = [[\"state_icon\", \"agent\"]]\n";
    let applied = add_quota_row(original).unwrap();
    assert!(applied.contains("key = \"prefix+shift+r\""));
    assert!(applied.contains("type = \"plugin_action\""));
    assert!(applied.contains("command = \"herdr-agent-quota.refresh\""));
    assert_eq!(add_quota_row(&applied).unwrap(), applied);
    assert_eq!(remove_quota_row(&applied).unwrap(), original);
}

#[test]
fn sidebar_configuration_preserves_a_conflicting_refresh_key() {
    let original = concat!(
        "[[keys.command]]\n",
        "key = \"prefix+shift+r\"\n",
        "type = \"shell\"\n",
        "command = \"echo user-owned\"\n",
        "description = \"user refresh\"\n\n",
        "[ui.sidebar.agents]\n",
        "rows = [[\"state_icon\", \"agent\"]]\n"
    );
    let applied = add_quota_row(original).unwrap();
    assert_eq!(applied.matches("key = \"prefix+shift+r\"").count(), 1);
    assert!(applied.contains("command = \"echo user-owned\""));
    assert!(!applied.contains("command = \"herdr-agent-quota.refresh\""));
    assert_eq!(remove_quota_row(&applied).unwrap(), original);
}

#[test]
fn default_herdr_rows_become_plane_provider_usage_and_topic_lines() {
    let original = concat!(
        "[ui.sidebar.agents]\n",
        "rows = [[\"state_icon\", \"workspace\", \"tab\"], [\"agent\"]]\n"
    );
    let applied = add_quota_row(original).unwrap();
    assert!(applied.contains("$quota_provider"));
    assert!(applied.contains("bold = true"));
    assert!(applied.contains("$quota_5h_normal"));
    assert!(applied.contains("$quota_5h_warning"));
    assert!(applied.contains("$quota_5h_danger"));
    assert!(applied.contains("$quota_week_normal"));
    assert!(applied.contains("$quota_week_warning"));
    assert!(applied.contains("$quota_week_danger"));
    assert!(!applied.contains("[\"$quota_summary\"]"));
    assert!(applied.contains("$quota_topic"));
    assert!(applied.contains("row_gap = 1 # herdr-agent-quota"));
    assert!(applied.find("$quota_topic").unwrap() < applied.find("$quota_5h_normal").unwrap());
    assert!(applied.contains("fg = \"#84b084\""));
    assert!(applied.contains("fg = \"#cdaa65\""));
    assert!(applied.contains("fg = \"#ca6470\""));
    assert!(applied.contains("[ui.sidebar.agents.rows_by_agent]"));
    assert!(applied.contains("fg = \"#c47f6a\""));
    assert!(applied.contains("fg = \"#7998b7\""));
    assert!(applied.contains("fg = \"#acb4c3\""));
    assert!(applied.contains("fg = \"#84b0af\""));
}

#[test]
fn sidebar_configuration_preserves_an_explicit_row_gap() {
    let original = concat!(
        "[ui.sidebar.agents]\n",
        "row_gap = 2\n",
        "rows = [[\"state_icon\", \"agent\"]]\n"
    );
    let applied = add_quota_row(original).unwrap();
    assert!(applied.contains("row_gap = 2"));
    assert!(!applied.contains("row_gap = 1"));
    assert_eq!(remove_quota_row(&applied).unwrap(), original);
}

#[test]
fn claude_collector_is_silent_without_a_previous_statusline() {
    let state = tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-agent-quota"))
        .arg("claude-statusline")
        .env("HERDR_PLUGIN_STATE_DIR", state.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(include_bytes!("fixtures/claude/statusline-both.json"))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn claude_cache_is_published_by_refresh_event() {
    let state = tempdir().unwrap();
    let (herdr_stub, herdr_log) = install_herdr_stub(
        state.path(),
        r#"{"result":{"agents":[{"agent":"claude","pane_id":"w1:p1"}]}}"#,
    );
    run_claude_collector(
        state.path(),
        &herdr_stub,
        include_bytes!("fixtures/claude/statusline-both.json"),
    );
    assert!(!herdr_log.exists());

    run_claude_refresh(state.path(), &herdr_stub);
    let report = fs::read_to_string(herdr_log).unwrap();
    assert!(!report.contains("pane read"));
    assert!(report.contains("quota_5h=5h 42% reset"));
    assert!(report.contains("quota_week=week 73% reset"));
}

#[test]
fn claude_collector_does_not_republish_unchanged_quota() {
    let state = tempdir().unwrap();
    let (herdr_stub, herdr_log) = install_herdr_stub(
        state.path(),
        r#"{"result":{"agents":[{"agent":"claude","pane_id":"w1:p1","tokens":{"quota_badge":"[A]","quota_state":"?","quota_icon":"✦Cl","quota_provider":"Claude","quota_status":"N/A","quota_5h":"5h 42%","quota_5h_warning":"5h 42%","quota_week":"week 73%","quota_week_warning":"week 73%","quota_summary":"5h 42% · week 73%"}}]}}"#,
    );

    let input = br#"{
        "rate_limits": {
            "five_hour": {"used_percentage": 58.0},
            "seven_day": {"used_percentage": 27.0}
        }
    }"#;
    run_claude_collector(state.path(), &herdr_stub, input);
    assert!(!herdr_log.exists());

    run_claude_refresh(state.path(), &herdr_stub);
    assert!(!herdr_log.exists());
}
