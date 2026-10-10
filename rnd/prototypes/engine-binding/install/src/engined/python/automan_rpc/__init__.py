"""PROTOTYPE thin SDK: find the `engined` binary shipped in this wheel and ping it over stdio."""
import json
import os
import shutil
import subprocess
import sys
import sysconfig

EXE = "engined" + (".exe" if sys.platform == "win32" else "")


def _user_scheme() -> str:
    if hasattr(sysconfig, "get_preferred_scheme"):
        return sysconfig.get_preferred_scheme("user")
    if os.name == "nt":
        return "nt_user"
    return "osx_framework_user" if sys.platform == "darwin" else "posix_user"


def find_engined() -> str:
    """Same search order as ruff's `find_ruff_bin`: venv scripts, user scheme, then PATH."""
    candidates = [os.path.join(sysconfig.get_path("scripts"), EXE)]
    try:
        candidates.append(os.path.join(sysconfig.get_path("scripts", scheme=_user_scheme()), EXE))
    except KeyError:
        pass
    for path in candidates:
        if os.path.isfile(path):
            return path
    found = shutil.which(EXE)
    if found:
        return found
    raise FileNotFoundError(
        "automan_rpc: could not find the `engined` binary. Looked in: "
        + ", ".join(candidates)
        + " and PATH. The wheel for this platform may be missing; reinstall automan-rpc."
    )


def ping() -> dict:
    proc = subprocess.run(
        [find_engined()],
        input='{"jsonrpc":"2.0","id":1,"method":"ping"}\n',
        capture_output=True,
        text=True,
        timeout=30,
        check=True,
    )
    return json.loads(proc.stdout.splitlines()[0])["result"]
