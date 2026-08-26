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


def _pane_id() -> str | None:
    if (os.environ.get("HERDR_ENV") or "").strip() != "1":
        return None
    return os.environ.get("HERDR_PANE_ID", "").strip() or None


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
    """Reconstruct the same Codex sidebar tokens herdr-agent-quota writes
    on a native codex pane, by reading its local snapshot file. This lets a
    Codex model running *inside* Hermes show an identical quota row."""
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

        def severity(remaining_pct: float, resets_at: int) -> str:
            remaining_seconds = max(resets_at - now_unix, 0)
            if remaining_seconds <= 0:
                return "unknown"
            remaining_time_pct = (
                min(remaining_seconds, duration_seconds(kind))
                / duration_seconds(kind)
                * 100.0
            )
            if remaining_pct >= remaining_time_pct:
                return "normal"
            if remaining_pct < 20.0:
                return "danger"
            return "warning"

        def fmt_reset(resets_at: int) -> str:
            secs = max(resets_at - now_unix, 0)
            minutes = max(secs // 60, 1)
            if minutes >= 24 * 60:
                return f"{minutes // (24 * 60)}d{minutes % (24 * 60) // 60}h"
            if minutes >= 60:
                return f"{minutes // 60}h{minutes % 60:02d}"
            return f"{minutes}m"

        for w in snap.get("windows", []):
            kind = w.get("kind")
            remaining = float(w.get("remaining_percent", 0))
            resets = int(w.get("resets_at", 0))
            label = window_label(kind)
            value = f"{label} {fmt_pct(remaining)}% {fmt_reset(resets)}"
            sev = severity(remaining, resets)
            key = "5h" if label == "5h" else "week"
            result[f"quota_{key}"] = value
            result[f"quota_{key}_{sev}"] = value

        ctx = snap.get("context") or {}
        if ctx:
            result["quota_context"] = f"context {fmt_pct(float(ctx.get('used_percent', 0)))}%"
            cache = ctx.get("cache") or {}
            hit = float(cache.get("hit_percent", 0))
            result["quota_cache"] = f"cache {hit:.1f}%"
            ttl = int(cache.get("ttl_seconds", 0))
            if ttl > 0:
                result["quota_cache_ttl"] = f"ttl≈{fmt_reset(int(now_unix + ttl))}"

        model = snap.get("model") or "Codex"
        result["quota_provider"] = "Codex"
        result["quota_provider_model"] = model

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
    if _uses_openrouter(model):
        tokens.update(_fetch_openrouter_credits())
    elif _is_codex(model):
        tokens.update(_codex_quota_tokens())
    return tokens


def _is_codex(model: str) -> bool:
    label = model.lower()
    return (
        label == "gpt-5.6-luna"
        or label.startswith("codex")
        or "codex" in label
    )


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


def _uses_openrouter(model: str | None = None) -> bool:
    """True when the active inference backend is OpenRouter (credits apply)."""
    label = (model or _model_label()).lower()
    if "openrouter" in label:
        return True
    # model id like "tencent/hy3" is served through OpenRouter.
    if "/" in label:
        return True
    return False


def _report_metadata(pane_id: str, model: str | None = None) -> None:
    tokens = _build_tokens(model)
    if not tokens:
        return
    herdr = os.environ.get("HERDR_BIN_PATH") or "herdr"
    cmd = [herdr, "pane", "report-metadata", pane_id, "--source", _SOURCE, "--agent", _AGENT]
    for name, value in tokens.items():
        cmd += ["--token", f"{name}={value}"]
    # Clear any tokens from a previous provider so stale quota rows don't
    # linger when switching between OpenRouter, Codex, etc.
    all_quota_tokens = {
        "quota_provider",
        "quota_provider_model",
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
    for name in all_quota_tokens:
        if name not in tokens:
            cmd += ["--clear-token", name]
    cmd += ["--seq", str(time.time_ns())]
    try:
        subprocess.run(
            cmd,
            check=False,
            timeout=1,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except Exception:
        pass


def _send_session(session_id: str, start_source: str, model: str | None = None) -> None:
    pane_id = _pane_id()
    if pane_id is None:
        return
    herdr = os.environ.get("HERDR_BIN_PATH") or "herdr"
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
        kwargs = {"timeout": 1, "stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
        if os.name == "nt":
            kwargs["creationflags"] = subprocess.CREATE_NO_WINDOW
        subprocess.run(command, check=False, **kwargs)
    except Exception:
        pass
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
    if isinstance(model, str) and model.strip():
        _send_session(session_id, start_source, model.strip())
    else:
        _send_session(session_id, start_source)


def _session_started(**kwargs) -> None:
    _report_session("startup", **kwargs)


def _session_reset(**kwargs) -> None:
    _report_session("new", **kwargs)


def _session_observed(**kwargs) -> None:
    if kwargs.get("platform") == "cli":
        _report_session("resume", **kwargs)


def register(ctx):
    ctx.register_hook("on_session_start", _session_started)
    ctx.register_hook("on_session_reset", _session_reset)
    ctx.register_hook("pre_llm_call", _session_observed)
