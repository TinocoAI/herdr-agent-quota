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
  hi                     ← what that pane is actually working on
  5h 100% reset 3h07m
  week 31% reset 2d3h
```

![Live Herdr agent sidebar](docs/screenshots/herdr-sidebar-live.png)

*A real Herdr workspace: Claude shows separate five-hour and weekly reset
ETAs, Codex and Grok show their weekly windows, and each agent card uses the
latest user prompt rather than an AI-generated status.*

- **Four CLIs, one sidebar** — Claude Code, Codex, Grok, Agy/Antigravity.
- **Three or four lines per pane** — provider, one line per quota window, and
  the latest user prompt.
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

### Time-aware quota health

Quota colors answer “will this allowance last until reset?” instead of applying
a fixed percentage threshold. For each available 5-hour or 7-day window, the
plugin computes:

```text
time_left  = (reset_at - now) / window_duration
quota_left = remaining_percent / 100
health     = quota_left / time_left
```

- **Green** — `health >= 1`: quota is being consumed no faster than time.
- **Amber** — `health < 1`: current usage is ahead of the sustainable pace.
- **Red** — `health < 1` and less than 20% quota remains: exhaustion risk is
  both immediate and material.
- **Amber fallback** — reset data is missing or expired, so the plugin avoids
  claiming that the quota is safe.

This explains the screenshot: Claude's weekly 24% is green because only about
13% of the week remains, while Grok's weekly 20% is amber because about 69% of
its window remains. The calculation is shared by every provider adapter; only
the window data differs.

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
per-window sidebar edit and installs a reversible Claude Code `statusLine`
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
| Claude Code `2.1.233` | `5h` + `week` | Official `statusLine` JSON: `rate_limits.five_hour` and `seven_day` | `configure --apply` chains an existing `statusLine` command |
| OpenAI Codex `0.147.0` | `week` | One-shot local `codex app-server --stdio`, `account/rateLimits/read` | ChatGPT subscription login; API-key mode is shown as unavailable |
| Grok CLI / Grok Build `1.0.3` | `week` | Local `~/.grok/auth.json` and the billing contract used by the official CLI | Log in to Grok CLI; no xAI team/API billing is queried |
| Agy / Antigravity CLI `1.1.13` | `5h` + `week` | Official `statusLine` JSON `quota` object (`gemini-*` and `3p-*` pools) | Set the native `/statusline` command once (below) |

Versions above were checked on the development machine on 2026-08-15. The
parser follows the provider fields rather than hard-coding these version
strings, so newer compatible CLI releases can continue to work.

The sidebar shows **percentage remaining** and the time until each reset, not
token counts. Codex and Grok expose their weekly window. Claude Code and Agy
expose both five-hour and weekly windows. Reset ETAs use minutes below one hour,
hours and minutes below one day, and days plus hours above one day. They are
recalculated on agent events or manual refresh; the sidebar does not run a
resident minute-by-minute countdown.

A failed refresh never replaces a successful cached value with `unavailable`;
a provider without any successful snapshot is shown as `N/A` until its first
usable event.

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
row_gap = 1 # herdr-agent-quota
rows = [
  ["state_icon", "tab", { token = "$quota_provider", bold = true, dim = false }],
  [{ token = "$quota_topic", dim = false }],
  [{ token = "$quota_5h_normal", fg = "#2e8b57", bold = true, dim = false }],
  [{ token = "$quota_5h_warning", fg = "#c47f00", bold = true, dim = false }],
  [{ token = "$quota_5h_danger", fg = "#d14343", bold = true, dim = false }],
  [{ token = "$quota_week_normal", fg = "#2e8b57", bold = true, dim = false }],
  [{ token = "$quota_week_warning", fg = "#c47f00", bold = true, dim = false }],
  [{ token = "$quota_week_danger", fg = "#d14343", bold = true, dim = false }],
]
```

- `state_icon` and `tab` are Herdr's built-in status and plane labels.
- `$quota_provider` is `Claude`, `Codex`, `Grok`, or `Agy`.
- Default provider labels use recognizable brand colors without affecting quota
  health: Claude coral-orange, Codex blue, adaptive monochrome for Grok, and a
  green from Antigravity's multicolor palette for Agy. Grok inherits the theme
  foreground, so it is white on dark themes and black on light themes.
- `$quota_topic` comes before the quota rows so the card reads as agent, task,
  then resource status.
- Each window publishes exactly one styled variant. Color follows runway rather
  than a fixed quota threshold: remaining quota is compared with the percentage
  of window time still left. At or ahead of pace is green; behind pace is
  amber; behind pace with less than 20% quota remaining is red. Missing or
  expired reset data uses the warning color. Herdr hides all absent variants,
  including the unsupported 5h row for Codex/Grok.
- `row_gap = 1` adds one blank row between agent cards. An existing explicit
  `row_gap` value is preserved.
- `$quota_5h`, `$quota_week`, and `$quota_summary` remain available for custom
  unstyled layouts.

Herdr 0.8 only accepts fixed hex colors for styled tokens, not semantic theme
colors. The green, slightly brighter amber, and red defaults use medium tones
and bold text so they remain distinguishable on common dark and light
backgrounds.

Provider styling uses Herdr's static `rows_by_agent` projection, while quota
health remains dynamic metadata. This keeps branding and health logic separate
and avoids spending additional metadata-token capacity on static labels.

Herdr plugin v1 accepts text tokens, not provider image components. For that
reason the default layout uses the readable provider name and keeps Herdr's
native dot instead of adding low-recognition Unicode or SVG markers. The
checked-in [`docs/icons/`](docs/icons/) assets are optional visual references;
they are not injected into the native sidebar.

The topic reader is event-driven: it scans recent pane output after an agent
event and extracts the latest user prompt. It deliberately leaves the topic
empty when no prompt is found instead of showing an AI-generated terminal title
such as `Thinking` or `Executing`. It does not show the working directory.

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
