# Hermes integration for herdr-agent-quota

This directory holds the **Hermes Agent** plugin that was built to make
`herdr-agent-quota` (this repo) useful when a Codex / ChatGPT model is run
*inside* Hermes instead of in a standalone Codex pane.

## The problem it solves

`herdr-agent-quota` (Rust) publishes `quota_*` sidebar tokens only for panes
where Herdr detects a native Codex agent. When you run Codex through Hermes
(`/model` -> `gpt-5.6-luna`, `codex/*`, etc.) the pane is still `hermes`, so the
Rust plugin never writes the quota tokens and the sidebar goes blank.

## The fix

`herdr_agent_state.py` is a Hermes plugin (Python). It:

- Reads the same local Codex subscription snapshot the Rust plugin writes
  (`~/.local/state/herdr/plugins/herdr-agent-quota/codex-app-server.json`).
- Reconstructs the **exact** `quota_*` tokens (`quota_5h`, `quota_week`,
  `quota_context`, `quota_cache`, `quota_topic`, `quota_provider`,
  `quota_provider_model`, plus the `*_normal`/`*_warning`/`*_danger` severity
  variants) and reports them to the `hermes` pane via `herdr pane report-metadata`.
- For OpenRouter-backed models (e.g. `tencent/hy3`, any `openrouter/...` id) it
  fetches live credit balance + percentage and reports `quota_credits` /
  `quota_credits_pct`.
- Clears stale tokens from the previous provider when you switch between
  OpenRouter and Codex, so the sidebar never shows a leftover row.

Result: running Codex inside Hermes shows the **same** quota row as a native
Codex pane.

## How it is wired into Hermes

The plugin registers Hermes lifecycle hooks (`on_session_start`,
`on_session_reset`, `on_session_observed`) and a `pre_llm_call` style hook that
receives `model=agent.model` — so a live `/model` change is reflected in the
sidebar without restarting anything that already has the new code loaded.

It is installed at `~/.hermes/plugins/herdr-agent-state/` (file `__init__.py`,
renamed to `herdr_agent_state.py` here for cleanliness) with the accompanying
`plugin.yaml`.

## Files

- `herdr_agent_state.py` — the plugin source (identical logic to the installed
  `~/.hermes/plugins/herdr-agent-state/__init__.py`, just renamed).
- `plugin.yaml` — Hermes plugin manifest (`name: herdr-agent-state`).
