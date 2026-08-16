#[test]
fn pane_focus_does_not_refresh_or_disturb_the_viewport() {
    let manifest = include_str!("../herdr-plugin.toml");
    assert!(!manifest.contains("on = \"pane.focused\""));
}
