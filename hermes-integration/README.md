# Hermes integration for herdr-agent-quota

Two small **Hermes Agent** plugins that make Hermes panes show up in Herdr's
agents panel and carry the `herdr-agent-quota` rows (backend, model, 5h/7d,
credits).

Tested with Hermes Agent v0.22.x (sessions run inside the local gateway
daemon) and Herdr 0.9.5.

## Files

- `herdr_agent_state.py` + `plugin.yaml`: install as
  `~/.hermes/plugins/herdr-agent-state/__init__.py` (+ `plugin.yaml`).
  It reports the Hermes **session id** of each pane to Herdr
  (`herdr pane report-agent-session`). The Rust plugin uses that id to look up
  the session in `~/.hermes/state.db` and publishes every `quota_*` row.
- `hermes-proctitle/`: install as `~/.hermes/plugins/hermes-proctitle/`.
  It sets the process title to `hermes` so Herdr's process detection
  recognises the pane.

Enable both and pass the pane variables through to gateway-run sessions:

```bash
hermes plugins enable herdr-agent-state
hermes plugins enable hermes-proctitle
hermes config set terminal.client_env_passthrough \
  '["HERDR_ENV","HERDR_PANE_ID","HERDR_SOCKET_PATH","HERDR_BIN_PATH","HERDR_SESSION","HERDR_WORKSPACE_ID","HERDR_TAB_ID"]'
```

Then restart the Hermes processes that are running in Herdr panes.

## Why each piece is needed

1. **Detection.** The Hermes source launcher execs `python3 -I -c <bootstrap>`,
   so argv0 is `python3` and Herdr never detects the pane as `hermes`. Hermes
   calls `setproctitle("hermes")` only when the module is importable, and
   `hermes update` recreates the venv, which removes a venv-installed
   `setproctitle`. `hermes-proctitle` loads it from a durable vendor dir
   (`~/.hermes/vendor/py<X.Y>-<arch>/`). If the module is missing there, it
   installs it in the background with Hermes' bundled `uv`.
   The title must be exactly `hermes`. On macOS, setproctitle writes the whole
   title into argv0 and Herdr matches argv0 exactly, so `hermes --resume X` is
   not detected.
2. **Sessions run in the gateway.** Since Hermes 0.22, CLI and TUI turns (and
   plugin hooks) run inside `hermes gateway run`, which has no `HERDR_*`
   variables. To find the pane that is showing a session right now, the plugin
   resolves in this order:
   1. Client registry plus breadcrumb. The `hermes` client in the pane writes
      `~/.hermes/cache/herdr-clients/tty-<dev>` with its `HERDR_*` values and
      pid. Hermes keeps a per-tty breadcrumb in `~/.hermes/terminal-sessions/`.
      Matching the two gives the current pane, also for `--resume`, `-c` and
      sessions created in another pane.
   2. The session's frozen client env (`terminal.client_env_passthrough`).
   3. `os.environ`.
   The plugin passes the pane's `HERDR_SOCKET_PATH` to `herdr`, so named Herdr
   sessions work too.
3. **One owner per token.** When the Rust plugin binary is present, this
   plugin publishes no `quota_*` tokens. If both publish, they conflict: this
   hook overwrote `quota_provider_model` with the bare model and cleared the
   5h/7d rows on every turn. Without the Rust plugin, it falls back to
   publishing Codex/OpenRouter tokens itself (the original behaviour), in
   batches of at most 16 tokens (the per-report limit in Herdr 0.9).

Failures are logged to `~/.hermes/logs/herdr-agent-state.log`. Hooks are
otherwise silent.

Note: `herdr integration install hermes` replaces
`~/.hermes/plugins/herdr-agent-state/__init__.py` with the upstream version.
Re-copy `herdr_agent_state.py` afterwards.
