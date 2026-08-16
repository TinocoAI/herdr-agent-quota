#[test]
fn pane_focus_uses_the_quota_only_focus_path() {
    let manifest = include_str!("../herdr-plugin.toml");
    let hook = manifest
        .split("[[events]]")
        .find(|event| event.contains("on = \"pane.focused\""))
        .unwrap();
    assert!(hook.contains(" focus\"]"));
    assert!(!hook.contains(" event\"]"));
}

#[test]
fn plugin_exposes_one_click_configure_and_uninstall_actions() {
    let manifest = include_str!("../herdr-plugin.toml");
    assert!(manifest.contains("id = \"configure\""));
    assert!(manifest.contains("configure --apply"));
    assert!(manifest.contains("id = \"uninstall\""));
    assert!(manifest.contains("configure --uninstall"));
}

#[test]
fn grok_runtime_refresh_does_not_go_through_a_plugin_action() {
    let manifest = include_str!("../herdr-plugin.toml");
    assert!(!manifest.contains("id = \"refresh-grok\""));
}
