#!/usr/bin/env python3
"""Type-check one writability run's program, the way the sketch's check.sh does.

Usage: check.py RUN_DIR [--quiet]

Run from the work folder's copy (WORK/bin/check.py). RUN_DIR holds task.json ({"kind": "gf" or
"leader", "lang": "ts", "py" or "rs", "program": file name, "env": the leader's environment,
"given": [file names]}) and the program. Every invocation is logged under RUN_DIR/.checks/; the
first one also keeps the program as it was then, the run's first attempt.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

T = Path(__file__).resolve().parent.parent
SKETCH = T / "sketch"
ENV = T / "env"
TSC = {k: T / k / "node_modules/.bin/tsc" for k in ("gf", "ld", "ld3")}
NODE_MODULES = {k: T / k / "node_modules" for k in ("gf", "ld", "ld3")}

# The sketch's tsconfig (typescript/tsconfig.json).
GF_TS_OPTIONS = {
    "target": "es2024",
    "module": "nodenext",
    "moduleResolution": "nodenext",
    "lib": ["es2024", "esnext.disposable", "dom"],
    "strict": True,
    "noUncheckedIndexedAccess": True,
    "exactOptionalPropertyTypes": True,
    "verbatimModuleSyntax": True,
    "erasableSyntaxOnly": True,
    "noEmit": True,
    "skipLibCheck": False,
    # check.sh compiles every scenario in one program, where other files' `/// <reference types="node" />`
    # makes Node's types global; a program checked alone gets them the same way.
    "types": ["node"],
}
# The leaders' tsconfig (scenarios/leaders/tsconfig.json).
LEADER_TS_OPTIONS = {
    "target": "es2024",
    "module": "nodenext",
    "moduleResolution": "nodenext",
    "lib": ["es2024", "dom"],
    "strict": True,
    "noEmit": True,
    "skipLibCheck": True,
    "types": ["node"],
}


def run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None, timeout: int = 900) -> tuple[int, str]:
    full_env = dict(os.environ)
    if env:
        full_env.update(env)
    try:
        proc = subprocess.run(cmd, cwd=cwd, env=full_env, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return 124, f"timed out after {timeout}s: {' '.join(cmd)}"
    return proc.returncode, proc.stdout + proc.stderr


def ts_paths() -> dict[str, list[str]]:
    base = SKETCH / "typescript"
    out = {}
    for pkg, folder in (("collie-client", "collie-client"), ("collie", "collie"), ("foxhound", "foxhound")):
        out[f"@goodfetch/{pkg}"] = [str(base / folder / "index.d.ts")]
        out[f"@goodfetch/{pkg}/testing"] = [str(base / folder / "testing.d.ts")]
    return out


def tsc(rundir: Path, files: list[str], options: dict, which: str, tag: str) -> tuple[int, str]:
    opts = dict(options)
    if which == "gf":
        opts["paths"] = ts_paths()
        opts["typeRoots"] = [str(NODE_MODULES["gf"] / "@types")]
    config = {"compilerOptions": opts, "files": [str(rundir / f) for f in files]}
    cfg = rundir / ".checks" / f"tsconfig.{tag}.json"
    cfg.write_text(json.dumps(config, indent=2))
    _, out = run([str(TSC[which]), "-p", str(cfg)], rundir)
    return len(re.findall(r"error TS\d+", out)), out


def pyright(rundir: Path, files: list[str], venv: str, mode: str, extra: list[str], tag: str) -> tuple[int, str]:
    config = {
        "typeCheckingMode": mode,
        "pythonVersion": "3.11",
        "extraPaths": [str(rundir)] + extra,
        "reportMissingModuleSource": "none",
        "venvPath": str(ENV / "py"),
        "venv": venv,
    }
    cfgdir = rundir / ".checks" / f"pyright.{tag}"
    cfgdir.mkdir(exist_ok=True)
    (cfgdir / "pyrightconfig.json").write_text(json.dumps(config, indent=2))
    _, out = run(
        [str(ENV / "py" / venv / "bin/pyright"), "-p", str(cfgdir / "pyrightconfig.json"), *[str(rundir / f) for f in files]],
        rundir,
    )
    if "No source files found" in out or "Ignoring path" in out:
        return 1, out
    m = re.search(r"(\d+) errors?,", out)
    return (int(m.group(1)) if m else 1), out


def mypy(rundir: Path, files: list[str]) -> tuple[int, str]:
    _, out = run(
        [
            str(ENV / "py/gf/bin/mypy"),
            "--strict",
            "--python-version",
            "3.11",
            "--enable-error-code",
            "deprecated",
            "--cache-dir",
            str(rundir / ".checks" / "mypy_cache"),
            *[str(rundir / f) for f in files],
        ],
        rundir,
        env={"MYPYPATH": f"{SKETCH / 'python'}:{rundir}"},
    )
    m = re.search(r"Found (\d+) errors?", out)
    return (int(m.group(1)) if m else 0), out


def cargo(rundir: Path, program: str, given: list[str], name: str) -> tuple[int, int, str]:
    exdir = ENV / "rust/examples" / name
    if exdir.exists():
        shutil.rmtree(exdir)
    exdir.mkdir(parents=True)
    shutil.copy(rundir / program, exdir / "main.rs")
    for g in given:
        shutil.copy(rundir / g, exdir / g)
    test_mode = bool(re.search(r"#\[(tokio::)?test", (rundir / program).read_text()))
    outs, errors, warnings = [], 0, 0
    for extra in [[]] + ([["--profile", "test"]] if test_mode else []):
        code, out = run(
            ["cargo", "check", "--example", name, "--all-features", "--message-format", "short", *extra],
            ENV / "rust",
            env={"CARGO_TERM_COLOR": "never"},
        )
        # Only the program's own diagnostics count: the stubs pass check.sh with warnings denied.
        lines = [l.replace(f"examples/{name}/main.rs", program) for l in out.splitlines() if f"examples/{name}/" in l]
        errors += sum(1 for l in lines if ": error" in l)
        warnings += sum(1 for l in lines if ": warning" in l)
        if code != 0 and not any(": error" in l for l in lines):
            errors += 1
            lines.append(out[-2000:])
        outs.append(("(test profile) " if extra else "") + "\n".join(lines))
    shutil.rmtree(exdir, ignore_errors=True)
    return errors, warnings, "\n".join(outs)


def check(rundir: Path, task: dict) -> dict:
    lang, kind, program = task["lang"], task["kind"], task["program"]
    given = task.get("given", [])
    if not (rundir / program).exists():
        return {"pass": False, "lenient_pass": False, "errors": 1, "output": f"{program} does not exist yet"}
    res: dict = {"lang": lang, "kind": kind}
    if lang == "ts":
        which = task.get("env", "gf") if kind == "leader" else "gf"
        link = rundir / "node_modules"
        if not link.exists():
            link.symlink_to(NODE_MODULES[which])
        if kind == "gf":
            e, out = tsc(rundir, [program] + given, GF_TS_OPTIONS, "gf", "strict")
            lenient = dict(LEADER_TS_OPTIONS, lib=GF_TS_OPTIONS["lib"], skipLibCheck=False)
            e2, _ = tsc(rundir, [program] + given, lenient, "gf", "lenient")
            res.update(errors=e, lenient_errors=e2, output=out, **{"pass": e == 0, "lenient_pass": e2 == 0})
        else:
            e, out = tsc(rundir, [program] + given, LEADER_TS_OPTIONS, which, "leader")
            res.update(errors=e, output=out, **{"pass": e == 0, "lenient_pass": e == 0})
    elif lang == "py":
        if kind == "gf":
            extra = [str(SKETCH / "python")]
            e1, o1 = pyright(rundir, [program], "gf", "strict", extra, "strict")
            e2, o2 = mypy(rundir, [program])
            e3, _ = pyright(rundir, [program], "gf", "standard", extra, "standard")
            res.update(pyright_errors=e1, mypy_errors=e2, lenient_errors=e3, errors=e1 + e2)
            res["output"] = "== pyright (strict) ==\n" + o1 + "\n== mypy (--strict) ==\n" + o2
            res["pass"] = e1 == 0 and e2 == 0
            res["lenient_pass"] = e3 == 0
        else:
            e, out = pyright(rundir, [program], task["env"], "standard", [], "leader")
            res.update(errors=e, output=out, **{"pass": e == 0, "lenient_pass": e == 0})
    else:
        name = "w_" + re.sub(r"[^a-z0-9]", "_", rundir.name.lower())
        e, w, out = cargo(rundir, program, given, name)
        res.update(errors=e, warnings=w, output=out or "cargo check: no diagnostics in the program")
        res["pass"] = e == 0 and w == 0
        res["lenient_pass"] = e == 0
    return res


def main() -> None:
    rundir = Path(sys.argv[1]).resolve()
    quiet = "--quiet" in sys.argv
    checks = rundir / ".checks"
    checks.mkdir(exist_ok=True)
    task = json.loads((rundir / "task.json").read_text())
    flat = task["program"].replace("/", "_")
    n = len(list(checks.glob("run-*.json"))) + 1
    if n == 1 and (rundir / task["program"]).exists():
        shutil.copy(rundir / task["program"], checks / ("first-" + flat))
    res = check(rundir, task)
    res.update(n=n, time=time.strftime("%Y-%m-%dT%H:%M:%S"), by=os.environ.get("CHECK_BY", "agent"))
    if (rundir / task["program"]).exists():
        shutil.copy(rundir / task["program"], checks / f"run-{n}-{flat}")
    (checks / f"run-{n}.json").write_text(json.dumps(res, indent=2))
    if quiet:
        print(json.dumps({k: v for k, v in res.items() if k != "output"}))
        return
    lines = res.get("output", "").splitlines()
    if len(lines) > 150:
        lines = lines[:150] + [f"... ({len(lines) - 150} more lines)"]
    print("\n".join(lines))
    print(f"\nCHECK {'PASSED' if res['pass'] else 'FAILED'} (check run {n})")


if __name__ == "__main__":
    main()
