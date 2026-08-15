use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Grok,
    Claude,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Grok, Self::Claude];

    pub fn badge(self) -> &'static str {
        match self {
            Self::Codex => "[C]",
            Self::Grok => "[X]",
            Self::Claude => "[A]",
        }
    }

    pub fn canonical_agent(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Claude => "claude",
        }
    }

    pub fn source(self) -> &'static str {
        match self {
            Self::Codex => "codex-app-server",
            Self::Grok => "grok-cli-billing",
            Self::Claude => "claude-statusline",
        }
    }
}

impl std::str::FromStr for Provider {
    type Err = ModelError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "grok" => Ok(Self::Grok),
            "claude" | "claude-code" | "anthropic" => Ok(Self::Claude),
            other => Err(ModelError::UnknownProvider(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    Weekly,
}

impl WindowKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::FiveHour => "5h",
            Self::Weekly => "wk",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    pub kind: WindowKind,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub resets_at: Option<String>,
}

impl UsageWindow {
    pub fn new(
        kind: WindowKind,
        used_percent: f64,
        resets_at: Option<String>,
    ) -> Result<Self, ModelError> {
        if !used_percent.is_finite() || !(0.0..=100.0).contains(&used_percent) {
            return Err(ModelError::InvalidPercentage(used_percent));
        }
        Ok(Self {
            kind,
            used_percent,
            remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
            resets_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub provider: Provider,
    pub source: String,
    pub fetched_at_unix: u64,
    pub windows: Vec<UsageWindow>,
}

impl ProviderSnapshot {
    pub fn new(provider: Provider, windows: Vec<UsageWindow>, fetched_at_unix: u64) -> Self {
        Self {
            provider,
            source: provider.source().to_string(),
            fetched_at_unix,
            windows,
        }
    }

    pub fn window(&self, kind: WindowKind) -> Option<&UsageWindow> {
        self.windows.iter().find(|window| window.kind == kind)
    }

    pub fn summary(&self) -> String {
        match self.provider {
            Provider::Codex | Provider::Grok => self
                .window(WindowKind::Weekly)
                .map(|window| format!("wk {}% left", format_percent(window.remaining_percent)))
                .unwrap_or_else(|| "wk unavailable".to_string()),
            Provider::Claude => {
                let five_hour = self
                    .window(WindowKind::FiveHour)
                    .map(|window| format!("5h {}% left", format_percent(window.remaining_percent)))
                    .unwrap_or_else(|| "5h unavailable".to_string());
                let weekly = self
                    .window(WindowKind::Weekly)
                    .map(|window| format!("wk {}% left", format_percent(window.remaining_percent)))
                    .unwrap_or_else(|| "wk unavailable".to_string());
                format!("{five_hour} · {weekly}")
            }
        }
    }

    pub fn severity(&self) -> Severity {
        let relevant = match self.provider {
            Provider::Codex | Provider::Grok => self.window(WindowKind::Weekly),
            Provider::Claude => self
                .window(WindowKind::FiveHour)
                .or_else(|| self.window(WindowKind::Weekly)),
        };
        relevant
            .map(|window| Severity::from_remaining(window.remaining_percent))
            .unwrap_or(Severity::Unknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Normal,
    Warning,
    Danger,
    Unknown,
}

impl Severity {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Normal => "●",
            Self::Warning => "▲",
            Self::Danger => "!",
            Self::Unknown => "?",
        }
    }

    pub fn from_remaining(remaining_percent: f64) -> Self {
        if remaining_percent > 30.0 {
            Self::Normal
        } else if remaining_percent >= 10.0 {
            Self::Warning
        } else {
            Self::Danger
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataTokens {
    pub quota_badge: String,
    pub quota_state: String,
    pub quota_summary: String,
    pub quota_error: Option<String>,
}

impl MetadataTokens {
    pub fn from_snapshot(snapshot: &ProviderSnapshot) -> Self {
        Self {
            quota_badge: snapshot.provider.badge().to_string(),
            quota_state: snapshot.severity().symbol().to_string(),
            quota_summary: snapshot.summary(),
            quota_error: None,
        }
    }

    pub fn unavailable(provider: Provider, reason: impl Into<String>) -> Self {
        Self {
            quota_badge: provider.badge().to_string(),
            quota_state: Severity::Unknown.symbol().to_string(),
            quota_summary: "unavailable".to_string(),
            quota_error: Some(reason.into().chars().take(80).collect()),
        }
    }
}

pub fn format_percent(value: f64) -> String {
    if (value - value.round()).abs() < f64::EPSILON {
        format!("{}", value.round() as u64)
    } else {
        format!("{value:.1}")
    }
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("percentage must be finite and between 0 and 100, got {0}")]
    InvalidPercentage(f64),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(kind: WindowKind, used: f64) -> UsageWindow {
        UsageWindow::new(kind, used, None).expect("fixture percentage is valid")
    }

    #[test]
    fn remaining_percentage_is_derived_from_used_percentage() {
        let value = window(WindowKind::Weekly, 42.5);
        assert_eq!(value.remaining_percent, 57.5);
    }

    #[test]
    fn severity_uses_remaining_percentage_boundaries() {
        assert_eq!(Severity::from_remaining(30.01), Severity::Normal);
        assert_eq!(Severity::from_remaining(30.0), Severity::Warning);
        assert_eq!(Severity::from_remaining(10.0), Severity::Warning);
        assert_eq!(Severity::from_remaining(9.99), Severity::Danger);
    }

    #[test]
    fn claude_summary_contains_both_windows_and_no_timestamp() {
        let snapshot = ProviderSnapshot::new(
            Provider::Claude,
            vec![
                window(WindowKind::FiveHour, 58.0),
                window(WindowKind::Weekly, 27.0),
            ],
            1,
        );
        assert_eq!(snapshot.summary(), "5h 42% left · wk 73% left");
        assert!(!snapshot.summary().contains("2026"));
    }

    #[test]
    fn provider_aliases_are_explicit() {
        assert_eq!("claude-code".parse::<Provider>().unwrap(), Provider::Claude);
        assert_eq!(Provider::Grok.badge(), "[X]");
    }

    #[test]
    fn unavailable_error_fits_herdr_token_limit() {
        let values = MetadataTokens::unavailable(Provider::Grok, "x".repeat(120));
        assert_eq!(values.quota_error.as_deref().unwrap().len(), 80);
    }
}
