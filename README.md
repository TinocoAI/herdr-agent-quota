# herdr-agent-quota

Show Claude Code, Codex, and Grok subscription usage in Herdr's agent sidebar.

This Rust plugin keeps the latest successful provider snapshot in a small local
cache and publishes compact text tokens to Herdr agent rows. It does not run a
resident daemon: Herdr startup, agent events, or a manual refresh perform a
one-shot update. If a later refresh fails, the previous value remains visible.

## What it shows

```text
[C] ● wk 39% left
[X] ▲ wk 21% left
[A] ● 5h 42% left · wk 73% left
```

The value is the percentage left in the provider's subscription window, not a
token count. `●` means more than 30% remains, `▲` means 10–30% remains, and `!`
means less than 10% remains. `? unavailable` means that provider has never
produced a usable snapshot on this machine.

The sidebar is intentionally text-based. Herdr plugin v1 accepts custom text
tokens but cannot inject brand SVG/PNG icons into its Agent renderer. The badges
are `[C]` Codex, `[X]` Grok, and `[A]` Claude/Anthropic.

## Data sources and privacy

- Codex: the local `codex app-server --stdio` JSON-RPC account/rate-limits
  contract. Only the seven-day window is shown. API-key mode is reported as
  unavailable because it is not the ChatGPT subscription quota.
- Grok: the local `~/.grok/auth.json` login key is read in memory and sent to the
  Grok CLI billing backend used by Grok Build. The response is accepted only when
  its period is explicitly weekly. This is SuperGrok subscription usage, not xAI
  developer/API-team billing.
- Claude Code: the official `statusLine` JSON hook supplies five-hour and
  seven-day windows. `configure --apply` backs up and chains an existing
  statusLine command.

The plugin does not access browser Cookies or browser Keychains, does not upload
usage data, does not refresh provider credentials, and does not save bearer or
refresh tokens. Provider failures never replace a successful cached snapshot.

The Grok CLI billing endpoint is an internal CLI contract rather than a public
stability guarantee. If xAI changes it, the plugin leaves the previous value in
place and reports the failure in the detail pane/action output.

## Build and local link

Requirements: Herdr `v0.8.0+`, Rust `1.95.0`, Codex `0.147.0`, and Claude Code
`v2.1.233+` for the tested contracts. macOS and Linux are supported.

```sh
cargo test --locked
cargo build --release --locked
herdr plugin link .
```

Then use the Herdr plugin action `Refresh agent quota` or run:

```sh
./target/release/herdr-agent-quota configure --check
./target/release/herdr-agent-quota configure --apply
```

`configure --apply` makes a small, idempotent sidebar-row edit and installs the
reversible Claude statusLine wrapper. `configure --uninstall` removes only the
plugin-owned row and restores the previous Claude statusLine. The manual sidebar
row is:

```toml
[ui.sidebar.agents]
rows = [
  ["state_icon", "agent"],
  ["$quota_badge", "$quota_state", "$quota_summary"],
]
```

The optional `Agent quota` pane is read-only. Press `r` to force one refresh and
`q` to close it; it does not poll.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
```

See [`docs/research/codexbar-grok-usage.md`](docs/research/codexbar-grok-usage.md)
for the Grok source investigation and
[`docs/plans/herdr-agent-quota-implementation.md`](docs/plans/herdr-agent-quota-implementation.md)
for the implementation contract.

## License

MIT. This project is not affiliated with Herdr, OpenAI, Anthropic, or xAI.
