# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- The Claude Code status line no longer goes blank when the plugin wrapper is
  installed on a machine that had no previous `statusLine` command. It now
  falls back to printing its own quota summary, always as exactly one line so
  the status line cannot oscillate in height and shift the pane.
- A pane that exits between `herdr agent list` and the metadata report no
  longer aborts the whole publish, so the remaining live panes still update.
  This was reachable in normal use, because `pane.exited` itself triggers a
  publish.
- Closed a race in the Codex app-server watchdog that could signal an unrelated
  process after the child had been reaped and its pid recycled. The watchdog
  and the request thread now share the child and terminate it at most once.
- A failed cache rename no longer leaves its scratch file behind.

- Throttled the Claude statusLine hook's sidebar publish to once every 30
  seconds. Claude re-runs its statusLine command constantly, and the hook was
  spawning six `herdr` subprocesses per tick — including a `report-metadata`
  aimed at the pane Claude was painting into — taking most of a second, to
  re-send percentages that change a few times an hour.

### Changed

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
