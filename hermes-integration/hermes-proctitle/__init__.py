"""Set the Hermes process title to ``hermes`` so Herdr detects it as an agent.

Why: the Hermes source launcher execs ``python3 -I -c <bootstrap>``, so argv0 is
``python3`` and Herdr's process detection (argv0 ``hermes``) never sees it -> no
entry in the agents panel and no quota rows. Hermes' main() calls
``setproctitle("hermes")`` only if the module is importable, and every
``hermes update`` that recreates the install venv drops it.

This plugin lives in ~/.hermes/plugins (untouched by updates) and loads
setproctitle from a durable vendor dir, ~/.hermes/vendor/py<X.Y>-<arch>.
If the vendor dir is missing (e.g. Python version bump), it installs it in the
background with Hermes' bundled uv so the NEXT launch is fixed automatically.
"""

from __future__ import annotations

import glob
import os
import platform
import subprocess
import sys

_HOME = os.environ.get("HERMES_HOME") or os.path.join(os.path.expanduser("~"), ".hermes")
_VENDOR = os.path.join(
    os.path.expanduser("~"), ".hermes", "vendor",
    f"py{sys.version_info[0]}.{sys.version_info[1]}-{platform.machine()}",
)


def _load():
    try:
        import setproctitle  # type: ignore
        return setproctitle
    except ImportError:
        pass
    if os.path.isdir(os.path.join(_VENDOR, "setproctitle")):
        if _VENDOR not in sys.path:
            sys.path.append(_VENDOR)
        try:
            import setproctitle  # type: ignore
            return setproctitle
        except ImportError:
            pass
    return None


def _install_in_background() -> None:
    uv = next(iter(sorted(glob.glob(os.path.expanduser("~/.hermes/tools/uv-*/uv*")), reverse=True)), None)
    if not uv or not os.access(uv, os.X_OK):
        return
    env = dict(os.environ)
    bundle = os.path.expanduser("~/.hermes/ssl/custom_bundle.pem")
    if os.path.isfile(bundle):
        env.setdefault("SSL_CERT_FILE", bundle)
    try:
        os.makedirs(_VENDOR, exist_ok=True)
        subprocess.Popen(
            [uv, "pip", "install", "--quiet", "--python", sys.executable, "--target", _VENDOR, "setproctitle"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            env=env, start_new_session=True,
        )
    except Exception:
        pass


def _apply() -> None:
    mod = _load()
    if mod is None:
        _install_in_background()
        return
    try:
        # Herdr matches argv0 EXACTLY against "hermes": on macOS setproctitle
        # writes the whole title into argv0, so never append the subcommand
        # ("hermes --resume ..." is not detected).
        if mod.getproctitle() != "hermes":
            mod.setproctitle("hermes")
    except Exception:
        pass


_apply()


def register(ctx):  # noqa: D401 - plugin entry point; work is done at import
    return None
