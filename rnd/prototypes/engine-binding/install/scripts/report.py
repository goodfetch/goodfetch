"""Prints the full matrix as Markdown and copies trimmed per-cell logs into logs/.

usage: report.py OUT_DIR LOGS_DIR
"""
import json
import re
import shutil
import sys
from pathlib import Path

out, logs = Path(sys.argv[1]), Path(sys.argv[2])
res = out / "results"
if logs.exists():
    shutil.rmtree(logs)
(logs / "cells").mkdir(parents=True)

MAX_LINES = 120


def kv(p: Path) -> dict:
    d = {}
    for line in p.read_text(errors="replace").splitlines():
        k, _, v = line.partition("=")
        d[k] = v.strip()
    return d


def first_error(cell: Path, install_ok: bool) -> str:
    src = cell / ("test.txt" if install_ok else "log.txt")
    if not src.exists():
        return ""
    lines = [l for l in src.read_text(errors="replace").splitlines() if not l.startswith("$ ")]
    best = re.compile(r"^\s*([A-Za-z]*Error\b|error\b|ERROR|ERR_|➤ YN0(?!000)|× |  × |caused by|Caused by)")
    generic = re.compile(r"Error|error|Cannot|No such|not found|denied|No matching|ERR_|ENOENT|EACCES|failed", re.I)
    for pat in (best, generic):
        for line in lines:
            if pat.search(line) and "throw" not in line:
                return line.strip()[:200]
    return ""


rows = []
for cell in sorted(res.iterdir()):
    if not (cell / "kv").exists():
        continue
    d = kv(cell / "kv")
    name = cell.name
    install_ok = d.get("install_rc") == "0"
    use_ok = d.get("test_rc") == "0"
    strategy = "rpc" if re.search(r"(^|[:-])rpc", name) else "native"
    rows.append({
        "cell": name, "strategy": strategy, "install": "ok" if install_ok else f"FAIL ({d.get('install_rc')})",
        "first_use": "ok" if use_ok else "FAIL", "secs": d.get("secs", ""), "rx_bytes": d.get("rx_bytes", ""),
        "disk_kb": d.get("disk_kb", ""), "installed": d.get("installed", ""),
        "tool": d.get("pm_version", ""), "runtime": d.get("runtime", ""),
        "error": "" if (install_ok and use_ok) else first_error(cell, install_ok),
        "gatekeeper": d.get("gatekeeper", ""),
    })
    log = (cell / "log.txt").read_text(errors="replace").splitlines() if (cell / "log.txt").exists() else []
    if len(log) > MAX_LINES:
        log = log[:60] + [f"... [{len(log) - MAX_LINES} lines trimmed] ..."] + log[-60:]
    safe = re.sub(r"[^A-Za-z0-9._+-]", "_", name)
    (logs / "cells" / f"{safe}.txt").write_text("\n".join(log) + "\n")
    shutil.copy(cell / "kv", logs / "cells" / f"{safe}.kv")

(logs / "matrix.json").write_text(json.dumps(rows, indent=1))
for f in ["build-linux.log", "build-macos.log", "npm-sizes.tsv"]:
    if (out / f).exists():
        lines = (out / f).read_text(errors="replace").splitlines()
        lines = [l for l in lines if "Downloaded " not in l and "Compiling " not in l]
        (logs / f).write_text("\n".join(lines[-400:]) + "\n")
if (out / "pyindex/sizes.json").exists():
    shutil.copy(out / "pyindex/sizes.json", logs / "pyindex-sizes.json")

print("| cell | install | first use | time (s) | rx bytes | on disk (KiB) | installed (npm pkgs / wheel tag) | tool | runtime | first error |")
print("|---|---|---|---|---|---|---|---|---|---|")
for r in rows:
    err = r["error"].replace("|", "\\|")
    print(f"| `{r['cell']}` | {r['install']} | {r['first_use']} | {r['secs']} | {r['rx_bytes']} | {r['disk_kb']} | "
          f"{r['installed'].strip()} | {r['tool']} | {r['runtime']} | {err} |")
print(f"\n{len(rows)} cells; install ok {sum(r['install']=='ok' for r in rows)}, "
      f"first use ok {sum(r['first_use']=='ok' for r in rows)}")
