# herdr-agent-quota

**Never hit a quota limit mid-task.** Live Claude Code, Codex, Grok, and
Agy/Antigravity subscription usage, in Herdr's agent sidebar.

[![CI](https://github.com/levi-qiao/herdr-agent-quota/actions/workflows/ci.yml/badge.svg)](https://github.com/levi-qiao/herdr-agent-quota/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Herdr plugin](https://img.shields.io/badge/Herdr-plugin-0.8%2B-5b6ee1)](https://herdr.dev/docs/plugins/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/levi-qiao/herdr-agent-quota?style=social)](https://github.com/levi-qiao/herdr-agent-quota)

中文文档：[README.zh-CN.md](README.zh-CN.md)

```text
● Owner · Claude
  5h 100% · week 31%     ← percent remaining, not token counts
  hi                     ← what that pane is actually working on
```

![Live Herdr agent sidebar](docs/screenshots/herdr-sidebar-live.png)

- **Four CLIs, one row each** — Claude Code, Codex, Grok, Agy/Antigravity.
- **Three lines per pane** — provider, remaining usage, current topic.
- **Local only** — no usage data uploaded, no browser cookies, no keychain
  scraping, and credentials are never written or refreshed.
- **Never lies to you** — a failed refresh keeps the last good number instead
  of flashing `unavailable`, and API-key auth is never shown as a subscription
  quota.
- **Fully reversible** — one command sets it up, one command puts your config
  back exactly as it was.

Install it in three commands ([quick start](#quick-start)):

```sh
herdr plugin link .
./target/release/herdr-agent-quota configure --apply
herdr server reload-config
```

The screenshot is a real local Herdr session. The values and topic text are
examples from that session; they are not hard-coded in the plugin.

## Quick start

Requirements: Herdr `0.8.0+`, Rust `1.95+`, macOS or Linux, and at least one
supported CLI. From this repository:

```sh
herdr plugin link .
./target/release/herdr-agent-quota configure --apply
herdr server reload-config
```

That is the complete Herdr setup. `herdr plugin link .` builds the Rust binary
and registers the startup/event hooks. `configure --apply` makes an idempotent
three-row sidebar edit and installs a reversible Claude Code `statusLine`
wrapper. You can run the same setup from Herdr's action menu with **Configure
agent quota sidebar**. Use **Refresh agent quota** for a one-shot refresh.

Preview the changes without writing anything:

```sh
./target/release/herdr-agent-quota configure --check
```

The setup preserves Herdr's native state dot and plane/tab label. It only adds
the provider, usage, and topic tokens, so the original Herdr agent indicator is
not removed. `configure --uninstall` removes the plugin-owned row and restores
the previous Claude statusLine.

## Supported CLIs

| CLI | Sidebar windows | Local collection path | Extra setup |
| --- | --- | --- | --- |
| Claude Code `2.1.232` | `5h` + `week` | Official `statusLine` JSON: `rate_limits.five_hour` and `seven_day` | `configure --apply` chains an existing `statusLine` command |
| OpenAI Codex `0.147.0` | `week` | One-shot local `codex app-server --stdio`, `account/rateLimits/read` | ChatGPT subscription login; API-key mode is shown as unavailable |
| Grok CLI / Grok Build `1.0.3` | `week` | Local `~/.grok/auth.json` and the billing contract used by the official CLI | Log in to Grok CLI; no xAI team/API billing is queried |
| Agy / Antigravity CLI `1.1.13` | `5h` + `week` | Official `statusLine` JSON `quota` object (`gemini-*` and `3p-*` pools) | Set the native `/statusline` command once (below) |

Versions above were checked on the development machine on 2026-08-15. The
parser follows the provider fields rather than hard-coding these version
strings, so newer compatible CLI releases can continue to work.

The sidebar shows **percentage remaining**, not token counts or reset times.
Codex and Grok expose their weekly window. Claude Code and Agy expose both
five-hour and weekly windows. A failed refresh never replaces a successful
cached value with `unavailable`; a provider without any successful snapshot is
shown as `N/A` until its first usable event.

## Agy / Antigravity setup

Agy sends its quota snapshot to the plugin through its native one-shot
`statusLine` hook. Set the command in Agy once:

```text
/statusline /absolute/path/to/herdr-agent-quota/target/release/herdr-agent-quota agy-statusline
```

The hook reads JSON from stdin, writes only sanitized percentages to the local
plugin cache, and exits. It is not a resident process and does not use browser
cookies or a private API.

## What the sidebar rows mean

The default rows are deliberately compact and keep the provider name only
once:

```toml
[ui.sidebar.agents]
rows = [
  ["state_icon", "tab", "$quota_provider"],
  ["$quota_summary"],
  ["$quota_topic"],
]
```

- `state_icon` and `tab` are Herdr's built-in status and plane labels.
- `$quota_provider` is `Claude`, `Codex`, `Grok`, or `Agy`.
- `$quota_summary` is the compact `5h ... · week ...` or `week ...` line.
- `$quota_topic` is the latest prompt/topic found in the pane output.

Herdr plugin v1 accepts text tokens, not provider image components. For that
reason the default layout uses the readable provider name and keeps Herdr's
native dot instead of adding low-recognition Unicode or SVG markers. The
checked-in [`docs/icons/`](docs/icons/) assets are optional visual references;
they are not injected into the native sidebar.

The topic reader is event-driven: it scans recent pane output after an agent
event, prefers the latest user prompt, and falls back to the native terminal
title when the CLI has not printed a prompt yet. It does not show the working
directory.

## Data sources and privacy

- **Codex:** the local official [app-server JSON-RPC](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md)
  rate-limit response. The plugin accepts the seven-day window by duration,
  rather than assuming which field is primary. API-key authentication is
  intentionally not mislabeled as a ChatGPT subscription quota.
- **Grok:** the local `~/.grok/auth.json` login key is read in memory and sent
  to the weekly billing endpoint used by the Grok CLI. The response is accepted
  only when it identifies a weekly period. This is SuperGrok usage, not xAI
  developer/API-team billing.
- **Claude Code:** the official [`statusLine` JSON hook](https://code.claude.com/docs/en/statusline)
  supplies the five-hour and seven-day values. A previous statusLine command is
  backed up, chained, and restored by `configure --uninstall`.
- **Agy/Antigravity:** the official [`/usage` and statusline docs](https://antigravity.google/docs/cli/commands/usage?app=antigravity-ide)
  supply Gemini and third-party pools. When both pools exist, the sidebar uses
  the lowest remaining percentage so the single Agy row is conservative.

Snapshots and refresh markers stay in Herdr's plugin state directory. No usage
data is uploaded, browser cookies or browser keychains are read, and provider
credentials are never refreshed or written. Provider failures leave the last
successful local value visible.

The Grok CLI billing endpoint is an internal CLI contract, not a public xAI
developer API stability promise. If it changes, the plugin keeps the previous
weekly value instead of clearing the sidebar.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| The rows do not appear | Run `herdr server reload-config`, then **Refresh agent quota**. |
| Claude or Agy is `N/A` | Start a conversation so the native `statusLine` emits JSON; then refresh. |
| Claude briefly changes while switching panes | The cached value is retained; run one prompt or a manual refresh if no snapshot exists yet. |
| Agy has no quota | Confirm the native `/statusline` command points to the built `agy-statusline` hook. |
| The topic is blank or old | Send a prompt in that pane; topic extraction runs on agent events and needs recent output. |
| Existing Claude statusLine is not changed | Run `configure --check`; the plugin refuses unsafe non-command settings instead of overwriting them. |

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
```

CI runs these on Linux and macOS for every pull request.

[`CONTRIBUTING.md`](CONTRIBUTING.md) covers the design rules every parser
follows and how to add a provider. Security reporting is in
[`SECURITY.md`](SECURITY.md), and released changes are in
[`CHANGELOG.md`](CHANGELOG.md).

The Grok source investigation is documented in
[`docs/research/codexbar-grok-usage.md`](docs/research/codexbar-grok-usage.md),
and the implementation contract is in
[`docs/plans/herdr-agent-quota-implementation.md`](docs/plans/herdr-agent-quota-implementation.md).

## Contributing

Adding a CLI is deliberately small: a pure `parse_*` function, a redacted
fixture, and a test. The rules it has to satisfy are in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

If this saved you a pane switch, a ⭐ helps other Herdr users find it. A bug
report with your CLI version is even better — it decides which provider parser
gets fixed next.

## License

MIT. This project is not affiliated with Herdr, OpenAI, Anthropic, xAI, or
Google.
