use herdr_agent_quota::dashboard::render_provider;
use herdr_agent_quota::model::{Provider, ProviderSnapshot, ResetAt, UsageWindow, WindowKind};

#[test]
fn agent_row_renders_compact_reset_eta_without_absolute_timestamp() {
    let snapshot = ProviderSnapshot::new(
        Provider::Grok,
        vec![UsageWindow::new(
            WindowKind::Weekly,
            79.0,
            Some(ResetAt::from_unix_seconds(183_600)),
        )
        .unwrap()],
        1,
    );
    assert_eq!(
        render_provider(Provider::Grok, Some(&snapshot), 0),
        "Grok WARN\r\n  7d 21% left reset 2d3h"
    );
}
