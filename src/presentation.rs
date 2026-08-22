use crate::model::{
    format_percent, Provider, ProviderSnapshot, ResetAt, Severity, UsageWindow, WindowKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataTokens {
    pub quota_state: String,
    pub quota_icon: String,
    pub quota_provider: String,
    pub quota_status: String,
    pub quota_5h: String,
    pub quota_5h_severity: Option<Severity>,
    pub quota_week: String,
    pub quota_week_severity: Option<Severity>,
    pub quota_summary: String,
    pub quota_context: String,
    pub quota_cache: String,
    pub quota_cache_ttl: String,
    pub quota_error: Option<String>,
}

impl MetadataTokens {
    pub fn from_snapshot(snapshot: &ProviderSnapshot, now_unix: u64) -> Self {
        Self {
            quota_state: snapshot.severity(now_unix).symbol().to_string(),
            quota_icon: snapshot.provider.icon().to_string(),
            quota_provider: snapshot.provider.display_name().to_string(),
            quota_status: snapshot.severity(now_unix).label().to_string(),
            quota_5h: sidebar_window(snapshot, WindowKind::FiveHour, now_unix),
            quota_5h_severity: window_severity(snapshot, WindowKind::FiveHour, now_unix),
            quota_week: sidebar_window(snapshot, WindowKind::Weekly, now_unix),
            quota_week_severity: window_severity(snapshot, WindowKind::Weekly, now_unix),
            quota_summary: sidebar_summary(snapshot, now_unix),
            quota_context: sidebar_context(snapshot),
            quota_cache: sidebar_cache(snapshot),
            quota_cache_ttl: sidebar_cache_ttl(snapshot, now_unix),
            quota_error: None,
        }
    }

    pub fn unavailable(provider: Provider, reason: impl Into<String>) -> Self {
        Self {
            quota_state: Severity::Unknown.symbol().to_string(),
            quota_icon: provider.icon().to_string(),
            quota_provider: provider.display_name().to_string(),
            quota_status: Severity::Unknown.label().to_string(),
            quota_5h: match provider {
                Provider::Claude | Provider::Agy => "5h N/A".to_string(),
                Provider::Codex | Provider::Grok => String::new(),
            },
            quota_5h_severity: match provider {
                Provider::Claude | Provider::Agy => Some(Severity::Unknown),
                Provider::Codex | Provider::Grok => None,
            },
            quota_week: "week N/A".to_string(),
            quota_week_severity: Some(Severity::Unknown),
            quota_summary: "unavailable".to_string(),
            quota_context: String::new(),
            quota_cache: String::new(),
            quota_cache_ttl: String::new(),
            quota_error: Some(reason.into().chars().take(80).collect()),
        }
    }
}

fn window_severity(
    snapshot: &ProviderSnapshot,
    kind: WindowKind,
    now_unix: u64,
) -> Option<Severity> {
    snapshot
        .window(kind)
        .map(|window| Severity::for_window(window, now_unix))
}

pub fn sidebar_summary(snapshot: &ProviderSnapshot, now_unix: u64) -> String {
    summary(snapshot, now_unix, false)
}

pub fn dashboard_summary(snapshot: &ProviderSnapshot, now_unix: u64) -> String {
    summary(snapshot, now_unix, true)
}

fn summary(snapshot: &ProviderSnapshot, now_unix: u64, include_left: bool) -> String {
    [WindowKind::FiveHour, WindowKind::Weekly]
        .into_iter()
        .filter_map(|kind| snapshot.window(kind))
        .map(|window| format_window(window, now_unix, include_left))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn sidebar_window(snapshot: &ProviderSnapshot, kind: WindowKind, now_unix: u64) -> String {
    snapshot
        .window(kind)
        .map(|window| format_window(window, now_unix, false))
        .unwrap_or_default()
}

fn sidebar_context(snapshot: &ProviderSnapshot) -> String {
    let Some(context) = snapshot.context.as_ref() else {
        return String::new();
    };
    format!("context {}%", format_percent(context.used_percent))
}

fn sidebar_cache(snapshot: &ProviderSnapshot) -> String {
    let Some(totals) = snapshot
        .context
        .as_ref()
        .and_then(|context| context.cache.as_ref())
        .and_then(|cache| cache.session_totals.as_ref())
    else {
        return String::new();
    };
    format!("cache {:.1}% hit", totals.hit_percent)
}

fn sidebar_cache_ttl(snapshot: &ProviderSnapshot, now_unix: u64) -> String {
    let Some(cache) = snapshot
        .context
        .as_ref()
        .and_then(|context| context.cache.as_ref())
    else {
        return String::new();
    };
    let Some(last_activity) = cache.last_activity_unix else {
        return String::new();
    };
    let elapsed = now_unix.saturating_sub(last_activity);
    let mut value = format!("cache last {} ago", format_elapsed(elapsed));
    if let Some(remaining) = cache.remaining_ttl_seconds(now_unix) {
        value.push_str(" · ttl≈");
        value.push_str(&format_ttl(remaining));
    }
    value
}

fn format_window(window: &UsageWindow, now_unix: u64, include_left: bool) -> String {
    let percent = format!("{}%", format_percent(window.remaining_percent));
    let left = if include_left { " left" } else { "" };
    let label = format!("{} {percent}{left}", window.kind.label());
    let Some(reset) = window.resets_at else {
        return label;
    };
    let eta = format_reset_eta(reset, now_unix);
    format!("{label} reset {eta}")
}

fn format_reset_eta(reset_at: ResetAt, now_unix: u64) -> String {
    let seconds = reset_at.unix_seconds().saturating_sub(now_unix);
    if seconds == 0 {
        return "due".to_string();
    }
    format_duration(seconds)
}

fn format_duration(seconds: u64) -> String {
    let minutes = (seconds / 60).max(1);
    if minutes >= 24 * 60 {
        return format!("{}d{}h", minutes / (24 * 60), (minutes % (24 * 60)) / 60);
    }
    if minutes >= 60 {
        return format!("{}h{:02}m", minutes / 60, minutes % 60);
    }
    format!("{minutes}m")
}

fn format_ttl(seconds: u64) -> String {
    if seconds == 0 {
        return "0m".to_string();
    }
    let minutes = seconds / 60;
    if (60..24 * 60).contains(&minutes) && minutes.is_multiple_of(60) {
        return format!("{}h", minutes / 60);
    }
    format_duration(seconds)
}

fn format_elapsed(seconds: u64) -> String {
    if seconds < 60 {
        return "<1m".to_string();
    }
    format_duration(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderSnapshot, UsageWindow};

    fn window(kind: WindowKind, used: f64, reset: u64) -> UsageWindow {
        UsageWindow::new(kind, used, Some(ResetAt::from_unix_seconds(reset))).unwrap()
    }

    #[test]
    fn formats_reset_eta_for_minutes_hours_days_and_due_windows() {
        assert_eq!(
            format_reset_eta(ResetAt::from_unix_seconds(2_700), 0),
            "45m"
        );
        assert_eq!(
            format_reset_eta(ResetAt::from_unix_seconds(14_820), 0),
            "4h07m"
        );
        assert_eq!(
            format_reset_eta(ResetAt::from_unix_seconds(183_600), 0),
            "2d3h"
        );
        assert_eq!(format_reset_eta(ResetAt::from_unix_seconds(99), 100), "due");
        assert_eq!(format_ttl(0), "0m");
        assert_eq!(format_ttl(3_600), "1h");
    }

    #[test]
    fn summary_is_window_driven_and_keeps_five_hour_before_weekly() {
        let snapshot = ProviderSnapshot::new(
            Provider::Claude,
            vec![
                window(WindowKind::Weekly, 27.0, 183_600),
                window(WindowKind::FiveHour, 58.0, 14_820),
            ],
            0,
        );
        assert_eq!(
            sidebar_summary(&snapshot, 0),
            "5h 42% reset 4h07m · week 73% reset 2d3h"
        );
        assert_eq!(
            dashboard_summary(&snapshot, 0),
            "5h 42% left reset 4h07m · week 73% left reset 2d3h"
        );
    }

    #[test]
    fn sidebar_windows_use_consistent_single_spacing() {
        let five_hour = format_window(&window(WindowKind::FiveHour, 57.0, 14_820), 0, false);
        let weekly = format_window(&window(WindowKind::Weekly, 75.0, 183_600), 0, false);
        assert_eq!(five_hour, "5h 43% reset 4h07m");
        assert_eq!(weekly, "week 25% reset 2d3h");
    }

    #[test]
    fn metadata_error_stays_within_herdr_token_limit() {
        let values = MetadataTokens::unavailable(Provider::Grok, "x".repeat(120));
        assert_eq!(values.quota_error.as_deref().unwrap().len(), 80);
    }

    #[test]
    fn metadata_keeps_severity_per_quota_window() {
        let snapshot = ProviderSnapshot::new(
            Provider::Claude,
            vec![
                window(WindowKind::FiveHour, 70.0, 14_820),
                window(WindowKind::Weekly, 90.0, 183_600),
            ],
            0,
        );
        let values = MetadataTokens::from_snapshot(&snapshot, 0);
        assert_eq!(values.quota_5h_severity, Some(Severity::Warning));
        assert_eq!(values.quota_week_severity, Some(Severity::Danger));
    }

    #[test]
    fn metadata_formats_context_usage_when_available() {
        let snapshot = ProviderSnapshot::new(
            Provider::Claude,
            vec![window(WindowKind::Weekly, 10.0, 183_600)],
            0,
        )
        .with_context(Some(crate::model::ContextUsage::new(23.5).unwrap()));
        let values = MetadataTokens::from_snapshot(&snapshot, 0);
        assert_eq!(values.quota_context, "context 24%");
    }

    #[test]
    fn metadata_formats_session_cache_hit_rate_and_approximate_ttl() {
        let cache = crate::model::CacheUsage::from_token_counts(100, 800, 100)
            .unwrap()
            .with_ttl_estimate(60 * 60, 0)
            .with_session_totals(
                crate::model::CacheTotals::from_token_counts(100, 800, 100),
                "session-1",
                1,
            );
        let context = crate::model::ContextUsage::new(23.5)
            .unwrap()
            .with_cache(Some(cache));
        let snapshot = ProviderSnapshot::new(
            Provider::Claude,
            vec![window(WindowKind::Weekly, 10.0, 183_600)],
            0,
        )
        .with_context(Some(context));
        let values = MetadataTokens::from_snapshot(&snapshot, 0);
        assert_eq!(values.quota_context, "context 24%");
        assert_eq!(values.quota_cache, "cache 80.0% hit");
        assert_eq!(values.quota_cache_ttl, "cache last <1m ago · ttl≈1h");
    }

    #[test]
    fn session_cache_percentage_keeps_one_decimal_instead_of_rounding_to_100() {
        let cache = crate::model::CacheUsage::from_token_counts(2_000, 433_336, 1_655)
            .unwrap()
            .with_session_totals(
                crate::model::CacheTotals::from_token_counts(2_000, 433_336, 1_655),
                "session-1",
                1,
            );
        let snapshot = ProviderSnapshot::new(Provider::Claude, vec![], 0).with_context(Some(
            crate::model::ContextUsage::new(43.0)
                .unwrap()
                .with_cache(Some(cache)),
        ));
        let values = MetadataTokens::from_snapshot(&snapshot, 0);
        assert_eq!(values.quota_cache, "cache 99.2% hit");
    }
}
