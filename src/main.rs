use anyhow::Result;
use clap::Parser;
use herdr_agent_quota::cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Refresh {
            provider,
            force,
            json,
        } => herdr_agent_quota::refresh::run(&provider.providers(), force, json),
        Command::Event => herdr_agent_quota::refresh::event(),
        Command::Dashboard => herdr_agent_quota::dashboard::run(),
        Command::Configure {
            check,
            apply,
            uninstall,
        } => herdr_agent_quota::configure::run(check, apply, uninstall),
        Command::ClaudeStatusline => herdr_agent_quota::configure::claude::run_statusline_hook(),
    }
}
