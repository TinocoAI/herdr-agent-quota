#[test]
fn pane_focus_refreshes_quota_that_changed_outside_herdr() {
    let manifest = include_str!("../herdr-plugin.toml");
    assert_eq!(manifest.matches("on = \"pane.focused\"").count(), 1);
}
