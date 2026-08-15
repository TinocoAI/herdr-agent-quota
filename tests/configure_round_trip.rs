use herdr_agent_quota::configure::herdr::{add_quota_row, remove_quota_row};

#[test]
fn sidebar_configuration_is_idempotent_and_reversible() {
    let original = "[ui.sidebar.agents]\nrows = [[\"state_icon\", \"agent\"]]\n";
    let applied = add_quota_row(original).unwrap();
    assert_eq!(add_quota_row(&applied).unwrap(), applied);
    assert_eq!(remove_quota_row(&applied).unwrap(), original);
}

#[test]
fn default_herdr_rows_become_plane_provider_usage_and_topic_lines() {
    let original = concat!(
        "[ui.sidebar.agents]\n",
        "rows = [[\"state_icon\", \"workspace\", \"tab\"], [\"agent\"]]\n"
    );
    let applied = add_quota_row(original).unwrap();
    assert!(applied.contains("[\"state_icon\", \"tab\", \"$quota_provider\"]"));
    assert!(applied.contains("[\"$quota_summary\"]"));
    assert!(applied.contains("[\"$quota_topic\"]"));
    assert_eq!(applied.matches("[\"").count(), 3);
}
