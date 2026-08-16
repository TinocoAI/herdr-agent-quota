# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Quota rows now show a compact `reset` ETA: minutes below one hour, hours and
  minutes below one day, and days plus hours for longer windows.

### Fixed

- Agent topics now come only from the latest user prompt in pane output. Native
  `Thinking`/`Executing` titles and other AI status text are no longer published
  as the user's topic, including Grok's `❯` prompt format.
- Codex Unix timestamps and Claude's Unix/RFC 3339 statusLine variants now
  normalize correctly; Grok RFC 3339 period ends and Agy relative reset seconds
  use the same cached absolute time.

- The Claude collector stays visually silent when there was no previous
  `statusLine`, avoiding a plugin-owned line that Claude repaints after each
  interaction. Existing custom status lines are still chained unchanged.
- A pane that exits between `herdr agent list` and the metadata report no
  longer aborts the whole publish, so the remaining live panes still update.
  This was reachable in normal use, because `pane.exited` itself triggers a
  publish.
- Closed a race in the Codex app-server watchdog that could signal an unrelated
  process after the child had been reaped and its pid recycled. The watchdog
  and the request thread now share the child and terminate it at most once.
- A failed cache rename no longer leaves its scratch file behind.

- Claude and Agy statusLine hooks now only update the local cache, so repainting
  an agent's own status line cannot synchronously call back into Herdr or move
  the terminal viewport. Metadata reports are skipped when every displayed
  token is unchanged.
- Focus changes now use a dedicated provider-only, 60-second-debounced refresh.
  This path never reads pane content or refreshes topics, and metadata writes
  remain suppressed while the selected pane is in scrollback.
- `configure --apply` now binds `prefix+shift+r` to the force-refresh action
  when that key is free, while preserving an existing user-owned binding.
  `configure --uninstall` removes only the plugin action binding.
- Grok now invokes a silent, provider-only refresh directly from `PostToolUse`
  during long-running turns, with turn-end hooks covering final, failed, and
  cancelled replies. It no longer routes these refreshes through a Herdr action,
  and remains debounced to avoid request storms.
- Quota-only refreshes no longer read every agent pane before publishing. They
  preserve the last topic token, update the sidebar as soon as quota collection
  finishes, and leave full topic extraction to agent lifecycle events.
- Agy's statusLine collector is now installed, repaired, chained, and removed
  by the same configuration lifecycle as Claude's, and remains silent when no
  user-owned status line existed.
- Configuration actions now install or uninstall all plugin-owned integrations
  in one pass and reload Herdr automatically. They also repair legacy
  collectors that pointed at a different cache directory while preserving any
  previous user statusLine backup.
- Metadata publication now skips panes whose viewport is in scrollback, so a
  Herdr repaint cannot pull the user back to the bottom. The next refresh after
  returning to the bottom catches the sidebar up.

### Changed

- Quota formatting is centralized in one presentation module shared by the
  sidebar, dashboard, and statusLine fallbacks. Codex remains weekly-only.
- Five-hour and weekly quota windows now occupy separate sidebar rows. Missing
  five-hour tokens are cleared so Herdr elides that row for Codex and Grok.
- Sidebar agent cards default to one blank row of separation, while preserving
  an existing `row_gap`. The latest user prompt now precedes compact,
  single-spaced quota rows, and percentages render as whole numbers.
- Default sidebar styling compares quota remaining with window time remaining:
  on-pace usage is bold green, behind-pace usage is brighter amber, and
  behind-pace usage below 20% remaining is bold red.
- Provider labels now use separate brand-aware `rows_by_agent` styling: Claude
  soft orange, Codex pastel blue, soft white for Grok, and Antigravity-inspired
  mint for Agy. Quota health colors use the same low-strain pastel palette.
  Existing user-owned agent row overrides remain untouched.

- Dropped the unmaintained `fs2` dependency in favour of the standard library's
  file locking, and made `libc` a Unix-only dependency.
- Removed the redundant `pkill` shell-out when tearing down the Codex
  app-server; killing the process group already covers its children.

## [0.1.0]

### Added

- Live Claude Code, Codex, Grok, and Agy/Antigravity subscription quotas in
  Herdr's agent sidebar, as five-hour and weekly remaining percentages.
- `configure --apply` / `--check` / `--uninstall` for a reversible, idempotent
  sidebar and Claude `statusLine` setup.
- A popup dashboard pane, event-driven refresh, and a local snapshot cache that
  survives provider failures.

[Unreleased]: https://github.com/levi-qiao/herdr-agent-quota/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/levi-qiao/herdr-agent-quota/releases/tag/v0.1.0
