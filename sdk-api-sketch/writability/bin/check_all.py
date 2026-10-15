#!/usr/bin/env python3
"""check_all.py PREFIX [JOBS] [--if-unchecked]: run the checker on every run folder whose name starts
with PREFIX; with --if-unchecked, only on runs never checked (an agent that never ran the check:
its first attempt is then its final program). Run from the work folder's copy."""
import json
import os
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

T = Path(__file__).resolve().parent.parent
args = [a for a in sys.argv[1:] if not a.startswith("--")]
dirs = sorted(p for p in (T / "runs").iterdir() if p.name.startswith(args[0]))
if "--if-unchecked" in sys.argv:
    dirs = [p for p in dirs if not list((p / ".checks").glob("run-*.json"))]


def one(p: Path) -> str:
    env = dict(os.environ, CHECK_BY="harness")
    out = subprocess.run([sys.executable, str(T / "bin/check.py"), str(p), "--quiet"], capture_output=True, text=True, env=env)
    try:
        d = json.loads(out.stdout.strip().splitlines()[-1])
    except (IndexError, json.JSONDecodeError):
        return f"{p.name} ?? {out.stdout[-300:]} {out.stderr[-500:]}"
    keys = ("errors", "warnings", "lenient_errors", "pyright_errors", "mypy_errors")
    return f"{p.name} {'PASS' if d['pass'] else 'FAIL'} " + json.dumps({k: d[k] for k in keys if k in d})


with ThreadPoolExecutor(int(args[1]) if len(args) > 1 else 2) as ex:
    for line in ex.map(one, dirs):
        print(line, flush=True)
