# Security policy

## Reporting a vulnerability

Report privately through
[GitHub Security Advisories](https://github.com/levi-qiao/herdr-agent-quota/security/advisories/new).
Please do not open a public issue for a vulnerability. Expect a first response
within 7 days.

## What this plugin touches

Useful context when judging impact:

- **Reads** `~/.grok/auth.json` (login key only), Claude Code and Agy
  statusLine JSON on stdin, and the local `codex app-server` JSON-RPC socket.
- **Writes** sanitized percentages to Herdr's plugin state directory,
  `~/.config/herdr/config.toml`, and `~/.claude/settings.json`. The last two
  are backed up and restored by `configure --uninstall`.
- **Sends** one authenticated request to the Grok CLI billing endpoint. This is
  the only outbound network call in the project. No usage data is uploaded
  anywhere.
- **Never** refreshes, rotates, or writes a provider credential, and never
  reads browser cookies or system keychains.

Credentials are held in memory for the duration of a single request and are
never written to the cache or logged.

## Supported versions

The latest release on `main`.
