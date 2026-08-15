use herdr_agent_quota::dashboard::render_provider;
use herdr_agent_quota::model::{Provider, ProviderSnapshot, UsageWindow, WindowKind};

#[test]
fn agent_row_is_compact_and_does_not_render_times() {
    let snapshot = ProviderSnapshot::new(
        Provider::Grok,
        vec![UsageWindow::new(
            WindowKind::Weekly,
            79.0,
            Some("2026-08-22T00:00:00Z".to_string()),
        )
        .unwrap()],
        1,
    );
    assert_eq!(
        render_provider(Provider::Grok, Some(&snapshot)),
        "[X] ▲ wk 21% left"
    );
}
