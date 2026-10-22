"""Hermes plugin installed by Herdr to report resumable session identity
and live inference provider / model / credit usage to the Herdr sidebar.
"""

# HERDR_INTEGRATION_ID=hermes
# HERDR_INTEGRATION_VERSION=5

from __future__ import annotations

import json
import os
import subprocess
import time
from urllib.request import Request, urlopen

_SOURCE = "herdr:hermes"
_AGENT = "hermes"
_INTERACTIVE_PLATFORMS = {"cli", "tui", "desktop", "acp"}
_MAX_TOKENS_PER_REPORT = 16  # Herdr 0.9.x limit per report-metadata call

# Cache the OpenRouter credit fetch so we don't hit the API on every LLM call.
_CREDIT_CACHE_TTL = 60.0
_credit_cache = [0.0, {}]  # [timestamp, result_dict]

# Path where herdr-agent-quota stores its local Codex subscription snapshot.
_CODEX_STATE = os.path.join(
    os.path.expanduser("~"),
    ".local",
    "state",
    "herdr",
    "plugins",
    "herdr-agent-quota",
    "codex-app-server.json",
)
_CODEX_TTL = 60.0
_codex_cache = [0.0, {}]  # [timestamp, result_dict]


def _log(msg: str) -> None:
    """Append failures to ~/.hermes/logs/herdr-agent-state.log (hooks are otherwise silent)."""
    try:
        path = os.path.join(os.path.expanduser("~"), ".hermes", "logs", "herdr-agent-state.log")
        if os.path.exists(path) and os.path.getsize(path) > 512_000:
            os.replace(path, path + ".1")
        with open(path, "a", encoding="utf-8") as fh:
            fh.write(f"{time.strftime('%Y-%m-%d %H:%M:%S')} pid={os.getpid()} pane={_client_env().get('HERDR_PANE_ID')} {msg}\n")
    except Exception:
        pass


_HERDR_VARS = ("HERDR_ENV", "HERDR_PANE_ID", "HERDR_SOCKET_PATH", "HERDR_BIN_PATH", "HERDR_SESSION",
               "HERDR_WORKSPACE_ID", "HERDR_TAB_ID")

import contextvars as _cv

# Session id of the hook currently running (gateway runs many sessions concurrently).
_CURRENT_SESSION: "_cv.ContextVar[str | None]" = _cv.ContextVar("herdr_current_session", default=None)
_HERMES_HOME = os.environ.get("HERMES_HOME") or os.path.join(os.path.expanduser("~"), ".hermes")
_REGISTRY_DIR = os.path.join(_HERMES_HOME, "cache", "herdr-clients")
_BREADCRUMB_DIR = os.path.join(_HERMES_HOME, "terminal-sessions")


def _tty_key() -> str | None:
    for fd in (0, 1):
        try:
            name = os.ttyname(fd)
        except Exception:
            continue
        if name:
            return "tty-" + name.strip("/").replace("/", "-")
    return None


def _register_client() -> None:
    """Client side (the `hermes` process in a Herdr pane, which HAS the pane's HERDR_*):
    record tty -> HERDR_* so the gateway can map a session to the pane it is shown in NOW
    (via Hermes' per-tty breadcrumbs), including resumed sessions whose frozen
    client_env still points at the pane that created them."""
    if (os.environ.get("HERDR_ENV") or "") != "1" or not os.environ.get("HERDR_PANE_ID"):
        return
    key = _tty_key()
    if not key:
        return
    try:
        os.makedirs(_REGISTRY_DIR, exist_ok=True)
        data = {k: os.environ[k] for k in _HERDR_VARS if os.environ.get(k)}
        data["pid"] = os.getpid()
        tmp = os.path.join(_REGISTRY_DIR, f".{key}.{os.getpid()}")
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump(data, fh)
        os.replace(tmp, os.path.join(_REGISTRY_DIR, key))
    except Exception:
        pass


def _pid_alive(pid) -> bool:
    try:
        os.kill(int(pid), 0)
        return True
    except PermissionError:
        return True
    except Exception:
        return False


def _env_from_breadcrumbs(session_id: str) -> dict | None:
    """Pane env of the live client currently showing ``session_id`` (newest breadcrumb wins)."""
    try:
        crumbs = []
        for name in os.listdir(_BREADCRUMB_DIR):
            try:
                with open(os.path.join(_BREADCRUMB_DIR, name), "r", encoding="utf-8") as fh:
                    d = json.load(fh)
            except Exception:
                continue
            if isinstance(d, dict) and d.get("session_id") == session_id:
                crumbs.append((float(d.get("ts") or 0), name))
        for _ts, name in sorted(crumbs, reverse=True):
            try:
                with open(os.path.join(_REGISTRY_DIR, name), "r", encoding="utf-8") as fh:
                    reg = json.load(fh)
            except Exception:
                continue
            if reg.get("HERDR_PANE_ID") and _pid_alive(reg.get("pid")):
                return {k: reg[k] for k in _HERDR_VARS if reg.get(k)}
    except Exception:
        pass
    return None


def _client_env() -> dict:
    """HERDR_* of the pane currently showing this session.

    Hermes >= 0.22 runs CLI/TUI sessions inside the gateway daemon, whose own
    environment has no HERDR_* vars. Resolution order:
    1. per-tty breadcrumb + client registry (correct for new AND resumed sessions);
    2. the session's frozen client env (``terminal.client_env_passthrough``);
    3. os.environ (in-process runs).
    """
    sid = _CURRENT_SESSION.get()
    if sid:
        env = _env_from_breadcrumbs(sid)
        if env:
            return env
    try:
        from agent.session_client_env import bound_client_env

        bound = bound_client_env()
    except Exception:
        bound = None
    if isinstance(bound, dict) and bound.get("HERDR_PANE_ID"):
        return {k: bound[k] for k in _HERDR_VARS if bound.get(k)}
    return {k: os.environ[k] for k in _HERDR_VARS if os.environ.get(k)}


_register_client()


def _herdr_bin() -> str:
    return _client_env().get("HERDR_BIN_PATH") or "herdr"


def _herdr_subprocess_env() -> dict:
    """Subprocess env so `herdr` talks to the pane's Herdr session (named sessions included)."""
    env = dict(os.environ)
    env.update(_client_env())
    return env


def _pane_id() -> str | None:
    env = _client_env()
    if (env.get("HERDR_ENV") or "").strip() != "1":
        return None
    return (env.get("HERDR_PANE_ID") or "").strip() or None


def _load_env_file(path: str) -> dict[str, str]:
    out: dict[str, str] = {}
    try:
        with open(path, "r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                k, _, v = line.partition("=")
                out[k.strip()] = v.strip()
    except Exception:
        pass
    return out


def _openrouter_key() -> str | None:
    key = (os.environ.get("OPENROUTER_API_KEY") or "").strip()
    if key:
        return key
    env = _load_env_file(os.path.join(os.path.expanduser("~"), ".hermes", ".env"))
    return (env.get("OPENROUTER_API_KEY") or "").strip() or None


def _model_label() -> str:
    """Exact model id the Hermes agent is running on (e.g. tencent/hy3)."""
    model = (os.environ.get("HERMES_INFERENCE_MODEL") or "").strip()
    if model:
        return model
    # Read default model from config.yaml without requiring PyYAML.
    try:
        cfg_path = os.path.join(os.path.expanduser("~"), ".hermes", "config.yaml")
        with open(cfg_path, "r", encoding="utf-8") as fh:
            text = fh.read()
        # Locate the model: block and grab the first non-empty `model:` line.
        in_model = False
        for line in text.splitlines():
            stripped = line.strip()
            if stripped.startswith("model:"):
                in_model = True
                continue
            if in_model:
                if stripped.startswith("default:"):
                    return stripped.split(":", 1)[1].strip()
                if not stripped or (stripped.startswith("provider:") is False and ":" in stripped and not stripped.startswith(" ")):
                    # left the model: block
                    in_model = False
    except Exception:
        pass
    return "hermes"


def _provider_short() -> str:
    """Short provider tag derived from the model id / config."""
    label = _model_label().lower()
    if label.startswith("codex") or "codex" in label:
        return "Codex"
    if label.startswith("openrouter/") or "openrouter" in label:
        return "OpenRouter"
    # model id like "tencent/hy3" -> show provider part
    if "/" in label:
        return label.split("/", 1)[0].capitalize()
    return "Hermes"


def _fetch_openrouter_credits() -> dict[str, str]:
    """Return {available, pct} strings from /api/v1/credits, or {} on failure."""
    now = time.time()
    cached_at, cached = _credit_cache
    if now - cached_at < _CREDIT_CACHE_TTL and cached:
        return cached
    key = _openrouter_key()
    if not key:
        return {}
    try:
        req = Request(
            "https://openrouter.ai/api/v1/credits",
            headers={"Authorization": f"Bearer {key}", "Accept": "application/json"},
        )
        with urlopen(req, timeout=5) as resp:
            data = __import__("json").loads(resp.read()).get("data", {})
        total = float(data.get("total_credits") or 0)
        used = float(data.get("total_usage") or 0)
        if total <= 0:
            result: dict[str, str] = {}
        else:
            available = total - used
            pct = (available / total) * 100.0
            result = {
                "quota_credits": f"${available:.2f}",
                "quota_credits_pct": f"{pct:.0f}%",
            }
    except Exception:
        result = {}
    _credit_cache[0] = now
    _credit_cache[1] = result
    return result


def _codex_quota_tokens() -> dict[str, str]:
    """Reconstruct the exact Codex sidebar tokens herdr-agent-quota writes
    on a native codex pane, by reading its local snapshot file. A Codex model
    running *inside* Hermes must show an identical quota row — same tokens,
    same strings, no extra OpenRouter credit info."""
    now = time.time()
    cached_at, cached = _codex_cache
    if now - cached_at < _CODEX_TTL and cached:
        return cached
    result: dict[str, str] = {}
    try:
        with open(_CODEX_STATE, "r", encoding="utf-8") as fh:
            snap = json.load(fh)
        now_unix = int(now)

        def fmt_pct(v: float) -> str:
            return f"{v:.0f}"

        def window_label(kind: str) -> str:
            return "5h" if kind == "five_hour" else "7d"

        def duration_seconds(kind: str) -> int:
            return 5 * 3600 if kind == "five_hour" else 7 * 24 * 3600

        # Severity is keyed on the *time* remaining vs the window length,
        # matching herdr-agent-quota's Severity::for_window.
        def severity(remaining_pct: float, resets_at: int) -> str:
            # Level-only scheme, same as herdr-agent-quota: >=50 green,
            # 20-50 amber, <20 red; expired/missing reset -> unknown.
            if resets_at - now_unix <= 0:
                return "unknown"
            if remaining_pct >= 50.0:
                return "normal"
            if remaining_pct >= 20.0:
                return "warning"
            return "danger"

        def fmt_reset(resets_at: int) -> str:
            secs = max(resets_at - now_unix, 0)
            if secs == 0:
                return "due"
            minutes = max(secs // 60, 1)
            if minutes >= 24 * 60:
                return f"{minutes // (24 * 60)}d{minutes % (24 * 60) // 60}h"
            if minutes >= 60:
                return f"{minutes // 60}h{minutes % 60:02d}"
            return f"{minutes}m"

        # Match herdr-agent-quota's compact window string: "5h 42% reset 4h07m".
        window_parts: list[str] = []
        worst_sev = "normal"
        for w in snap.get("windows", []):
            kind = w.get("kind")
            remaining = float(w.get("remaining_percent", 0))
            resets = int(w.get("resets_at", 0))
            label = window_label(kind)
            value = f"{label} {fmt_pct(remaining)}% reset {fmt_reset(resets)}"
            sev = severity(remaining, resets)
            key = "5h" if label == "5h" else "week"
            result[f"quota_{key}"] = value
            result[f"quota_{key}_{sev}"] = value
            window_parts.append(value)
            if sev == "danger":
                worst_sev = "danger"
            elif sev == "warning" and worst_sev != "danger":
                worst_sev = "warning"

        # quota_summary joins both windows exactly like the Rust sidebar_summary.
        if window_parts:
            result["quota_summary"] = " · ".join(window_parts)

        # quota_state is the global severity symbol (●/▲/!/?).
        result["quota_state"] = {
            "normal": "●",
            "warning": "▲",
            "danger": "!",
            "unknown": "?",
        }.get(worst_sev, "●")

        ctx = snap.get("context") or {}
        if ctx:
            result["quota_context"] = f"context {fmt_pct(float(ctx.get('used_percent', 0)))}%"
            cache = ctx.get("cache") or {}
            hit = float(cache.get("hit_percent", 0))
            result["quota_cache"] = f"cache {hit:.1f}%"
            ttl = int(cache.get("ttl_seconds", 0))
            if ttl > 0:
                result["quota_cache_ttl"] = f"ttl≈{fmt_reset(int(now_unix + ttl))}"

        # quota_provider_model carries the "Codex/<model>" prefix, identical
        # to herdr-agent-quota's provider_model_label.
        model = snap.get("model") or ""
        result["quota_provider"] = "Codex"
        result["quota_provider_model"] = f"Codex/{model}" if model else "Codex"

        # Latest session summary as the topic, matching the codex pane.
        summaries = snap.get("session_summaries") or {}
        if summaries:
            result["quota_topic"] = list(summaries.values())[-1]
    except Exception:
        result = {}
    _codex_cache[0] = now
    _codex_cache[1] = result
    return result


def _build_tokens(model_override: str | None = None) -> dict[str, str]:
    model = (model_override or "").strip() or _model_label()
    tokens: dict[str, str] = {
        "quota_provider_model": model,
        "quota_provider": _provider_short_for(model),
    }
    if _is_codex_model(model):
        tokens.update(_codex_quota_tokens())
    elif _uses_openrouter(model):
        tokens.update(_fetch_openrouter_credits())
    return tokens


def _is_codex(model: str) -> bool:
    return _is_codex_model(model)


def _provider_short_for(model: str) -> str:
    label = model.lower()
    # ChatGPT/Codex subscription models are reported without a provider
    # prefix by Hermes (for example, gpt-5.6-luna).
    if label == "gpt-5.6-luna" or label.startswith("codex"):
        return "Codex"
    if label.startswith("openrouter/") or "openrouter" in label:
        return "OpenRouter"
    if "/" in label:
        return label.split("/", 1)[0].capitalize()
    return "Hermes"


def _is_codex_model(model: str) -> bool:
    """True for ChatGPT/Codex subscription models.

    These carry no OpenRouter credit balance, so they must be tested before
    the OpenRouter check (a model id like `gpt-5.6-luna` still contains a
    "/", which would otherwise be mistaken for an OpenRouter route)."""
    label = model.lower()
    return (
        label == "gpt-5.6-luna"
        or label.startswith("codex")
        or "codex" in label
    )


def _uses_openrouter(model: str | None = None) -> bool:
    """True when the active inference backend is OpenRouter (credits apply)."""
    label = (model or _model_label()).lower()
    if _is_codex_model(label):
        return False
    if "openrouter" in label:
        return True
    # model id like "tencent/hy3" is served through OpenRouter.
    if "/" in label:
        return True
    return False


def _quota_plugin_installed() -> bool:
    """The Rust herdr-agent-quota plugin owns every quota_* row (backend label, model, 5h/7d,
    credits). Publishing them from here too makes the two sources fight: this hook would
    overwrite ``quota_provider_model`` with the bare model and --clear the 5h/7d rows the
    quota plugin just published. When it is installed, only report the session id."""
    import glob as _glob

    pattern = os.path.expanduser("~/.config/herdr/plugins/github/herdr-agent-quota-*/target/release/herdr-agent-quota")
    return bool(_glob.glob(pattern))


def _report_metadata(pane_id: str, model: str | None = None) -> None:
    if _quota_plugin_installed():
        return
    tokens = _build_tokens(model)
    if not tokens:
        return
    herdr = _herdr_bin()
    ops: list[list[str]] = [["--token", f"{name}={value}"] for name, value in tokens.items()]
    # Clear any tokens from a previous provider so stale quota rows don't
    # linger when switching between OpenRouter, Codex, etc.
    all_quota_tokens = {
        "quota_provider",
        "quota_provider_model",
        "quota_state",
        "quota_summary",
        "quota_credits",
        "quota_credits_pct",
        "quota_5h",
        "quota_5h_normal",
        "quota_5h_warning",
        "quota_5h_danger",
        "quota_week",
        "quota_week_normal",
        "quota_week_warning",
        "quota_week_danger",
        "quota_context",
        "quota_cache",
        "quota_cache_ttl",
        "quota_topic",
    }
    for name in sorted(all_quota_tokens):
        if name not in tokens:
            ops.append(["--clear-token", name])
    # Herdr >= 0.9 rejects a report that touches more than 16 tokens
    # (invalid_metadata_token) -> send in batches, each with its own seq.
    base = [herdr, "pane", "report-metadata", pane_id, "--source", _SOURCE, "--agent", _AGENT]
    for i in range(0, len(ops), _MAX_TOKENS_PER_REPORT):
        cmd = list(base)
        for op in ops[i:i + _MAX_TOKENS_PER_REPORT]:
            cmd += op
        cmd += ["--seq", str(time.time_ns())]
        try:
            r = subprocess.run(
                cmd,
                check=False,
                timeout=2,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=_herdr_subprocess_env(),
            )
            if r.returncode:
                _log(f"report-metadata rc={r.returncode} {(r.stderr or b'')[:300]!r}")
        except Exception as exc:
            _log(f"report-metadata exc={exc!r}")


def _send_session(session_id: str, start_source: str, model: str | None = None) -> None:
    pane_id = _pane_id()
    if pane_id is None:
        return
    herdr = _herdr_bin()
    command = [
        herdr,
        "pane",
        "report-agent-session",
        pane_id,
        "--source",
        _SOURCE,
        "--agent",
        _AGENT,
        "--seq",
        str(time.time_ns()),
        "--agent-session-id",
        session_id,
        "--session-start-source",
        start_source,
    ]
    try:
        kwargs = {"timeout": 2, "stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL,
                  "env": _herdr_subprocess_env()}
        if os.name == "nt":
            kwargs["creationflags"] = subprocess.CREATE_NO_WINDOW
        kwargs["stderr"] = subprocess.PIPE
        r = subprocess.run(command, check=False, **kwargs)
        if r.returncode:
            _log(f"report-agent-session rc={r.returncode} {(r.stderr or b'')[:300]!r}")
    except Exception as exc:
        _log(f"report-agent-session exc={exc!r}")
    # Surface the active model + OpenRouter credit usage in the sidebar.
    _report_metadata(pane_id, model)


def _report_session(start_source: str, **kwargs) -> None:
    if kwargs.get("platform") not in _INTERACTIVE_PLATFORMS:
        return
    session_id = kwargs.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        return
    # Diagnostic: record what the hook actually receives so we can verify
    # model changes made via /model inside a live Hermes session.
    try:
        with open(os.path.join(os.path.expanduser("~"), ".hermes", "plugin_model_trace.log"), "a", encoding="utf-8") as _fh:
            _fh.write(f"{time.time():.0f} src={start_source} model={kwargs.get('model')!r} agent_model={_model_label()}\n")
    except Exception:
        pass
    # `pre_llm_call` passes `model=agent.model`, which reflects any model
    # change made in the live session. Fall back to the config default.
    model = kwargs.get("model")
    token = _CURRENT_SESSION.set(session_id)
    try:
        if isinstance(model, str) and model.strip():
            _send_session(session_id, start_source, model.strip())
        else:
            _send_session(session_id, start_source)
    finally:
        _CURRENT_SESSION.reset(token)


def _session_started(**kwargs) -> None:
    _report_session("startup", **kwargs)


def _session_reset(**kwargs) -> None:
    _report_session("new", **kwargs)


def _session_observed(**kwargs) -> None:
    if kwargs.get("platform") == "cli":
        _report_session("resume", **kwargs)
    _kick_quota_refresh(kwargs.get("session_id"), min_gap=5.0)


# --- quota refresh kick -------------------------------------------------------
# Herdr does not see lifecycle transitions of gateway-run Hermes sessions (the
# pane stays `idle`), so herdr-agent-quota's pane.agent_status_changed handler
# never fires and the rows only refreshed on pane focus. Kick the Rust plugin's
# DEBOUNCED `refresh-hermes` action (60s floor per provider, model switches
# still refetch) at turn start/end and, throttled, during long tool loops.
_last_kick: dict[str, float] = {}


def _kick_quota_refresh(session_id, min_gap: float) -> None:
    if not isinstance(session_id, str) or not session_id or not _quota_plugin_installed():
        return
    now = time.monotonic()
    if now - _last_kick.get(session_id, -1e9) < min_gap:
        return
    _last_kick[session_id] = now
    token = _CURRENT_SESSION.set(session_id)
    try:
        if _pane_id() is None:
            return
        cmd = [_herdr_bin(), "plugin", "action", "invoke", "herdr-agent-quota.refresh-hermes"]
        env = _herdr_subprocess_env()
    finally:
        _CURRENT_SESSION.reset(token)
    try:
        subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL, env=env, start_new_session=True)
    except Exception as exc:
        _log(f"refresh-hermes exc={exc!r}")


def _turn_finished(**kwargs) -> None:
    _kick_quota_refresh(kwargs.get("session_id"), min_gap=2.0)


def _tool_finished(**kwargs) -> None:
    _kick_quota_refresh(kwargs.get("session_id"), min_gap=30.0)


def register(ctx):
    ctx.register_hook("on_session_start", _session_started)
    ctx.register_hook("on_session_reset", _session_reset)
    ctx.register_hook("pre_llm_call", _session_observed)
    ctx.register_hook("post_llm_call", _turn_finished)
    ctx.register_hook("post_tool_call", _tool_finished)
