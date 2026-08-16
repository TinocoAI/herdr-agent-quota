#[test]
fn pane_focus_does_not_refresh_or_disturb_the_viewport() {
    let manifest = include_str!("../herdr-plugin.toml");
    assert!(!manifest.contains("on = \"pane.focused\""));
}

#[test]
fn plugin_exposes_one_click_configure_and_uninstall_actions() {
    let manifest = include_str!("../herdr-plugin.toml");
    assert!(manifest.contains("id = \"configure\""));
    assert!(manifest.contains("configure --apply"));
    assert!(manifest.contains("id = \"uninstall\""));
    assert!(manifest.contains("configure --uninstall"));
}
