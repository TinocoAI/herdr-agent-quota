#[test]
fn pane_focus_does_not_trigger_a_refresh_feedback_loop() {
    let manifest = include_str!("../herdr-plugin.toml");
    assert!(!manifest.contains("on = \"pane.focused\""));
}
