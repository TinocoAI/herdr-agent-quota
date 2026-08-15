use crate::cache::CacheStore;
use crate::model::{Provider, ProviderSnapshot};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

pub fn run() -> Result<()> {
    let cache = CacheStore::from_env()?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        print_snapshot(&cache)?;
        return Ok(());
    }
    enable_raw_mode()?;
    let result = interactive(&cache);
    let _ = disable_raw_mode();
    result
}

fn interactive(cache: &CacheStore) -> Result<()> {
    loop {
        print!(
            "{}",
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
        );
        print!("{}", crossterm::cursor::MoveTo(0, 0));
        print_snapshot(cache)?;
        println!("\nr refresh  q quit");
        io::stdout().flush()?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => {
                        crate::refresh::run(&Provider::ALL, true, false)?;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn print_snapshot(cache: &CacheStore) -> Result<()> {
    println!("Herdr Agent Quota");
    println!("=================");
    for provider in Provider::ALL {
        let snapshot = cache.load(provider)?;
        println!("{}", render_provider(provider, snapshot.as_ref()));
    }
    Ok(())
}

pub fn render_provider(provider: Provider, snapshot: Option<&ProviderSnapshot>) -> String {
    match snapshot {
        Some(snapshot) => format!(
            "{} {} {}",
            provider.badge(),
            snapshot.severity().symbol(),
            snapshot.summary()
        ),
        None => format!("{} ? unavailable", provider.badge()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{UsageWindow, WindowKind};

    #[test]
    fn renders_compact_remaining_values_without_timestamps() {
        let snapshot = ProviderSnapshot::new(
            Provider::Claude,
            vec![
                UsageWindow::new(WindowKind::FiveHour, 58.0, Some("2026-08-15".into())).unwrap(),
                UsageWindow::new(WindowKind::Weekly, 27.0, Some("2026-08-22".into())).unwrap(),
            ],
            1,
        );
        let rendered = render_provider(Provider::Claude, Some(&snapshot));
        assert_eq!(rendered, "[A] ● 5h 42% left · wk 73% left");
        assert!(!rendered.contains("2026"));
    }
}
