use herdr_agent_quota::configure::herdr::{add_quota_row, remove_quota_row};

#[test]
fn sidebar_configuration_is_idempotent_and_reversible() {
    let original = "[ui.sidebar.agents]\nrows = [[\"state_icon\", \"agent\"]]\n";
    let applied = add_quota_row(original).unwrap();
    assert_eq!(add_quota_row(&applied).unwrap(), applied);
    assert_eq!(remove_quota_row(&applied).unwrap(), original);
}
