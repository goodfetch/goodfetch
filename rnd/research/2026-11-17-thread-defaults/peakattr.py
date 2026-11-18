#!/usr/bin/env python3
"""#2200 (e): #2198's sampler2.py capture, for every Foxhound process a command starts.

Usage: peakattr.py OUTDIR -- COMMAND...
Runs COMMAND as a child, so (Yama ptrace_scope 1) this process may read its
descendants' /proc/<pid>/task/<tid>/syscall (each thread's stack pointer).
Every 10 ms, for each live goodfetch-foxhound descendant, reads smaps_rollup;
at each new RSS peak of that process writes OUTDIR/p<k>.{smaps,status,pagemap,tasks}
exactly as sampler2.py does (k = the process's order of discovery), and on its
exit one line of OUTDIR/meta.jsonl: k, pid, start and end (monotonic s from
the command's start), peak kB, samples. Timings of the command are not used.
"""
import json, os, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sampler_lib as L  # noqa: E402

OUT = sys.argv[1]
CMD = sys.argv[sys.argv.index("--") + 1:]
FOX = "goodfetch-foxhound"
os.makedirs(OUT, exist_ok=True)


def children(pid):
    out = []
    try:
        for tid in os.listdir(f"/proc/{pid}/task"):
            try:
                with open(f"/proc/{pid}/task/{tid}/children") as f:
                    out += [int(c) for c in f.read().split()]
            except OSError:
                pass
    except OSError:
        pass
    return out


def descendants(root):
    out, todo = [], [root]
    while todo:
        for c in children(todo.pop()):
            out.append(c); todo.append(c)
    return out


def comm(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def tasks(pid):
    rows = []
    for tid in os.listdir(f"/proc/{pid}/task"):
        try:
            c = open(f"/proc/{pid}/task/{tid}/comm").read().strip().replace(" ", "_")
            sc = open(f"/proc/{pid}/task/{tid}/syscall").read().split()
        except OSError:
            continue
        sp, pc = (sc[-2], sc[-1]) if len(sc) >= 3 else ("?", "?")
        rows.append(f"{tid} {c} {sp} {pc}")
    return "\n".join(rows) + "\n"


def main():
    t0 = time.monotonic()
    child = subprocess.Popen(CMD)
    procs = {}  # pid -> dict(k, start, peak, samples)
    done = set()
    meta = open(os.path.join(OUT, "meta.jsonl"), "w")

    def finish(pid):
        p = procs.pop(pid)
        done.add(pid)
        meta.write(json.dumps({"k": p["k"], "pid": pid, "start": round(p["start"], 3),
                               "end": round(time.monotonic() - t0, 3), "peak_kb": p["peak"],
                               "samples": p["samples"]}) + "\n")
        meta.flush()

    k = 0
    while child.poll() is None:
        for pid in descendants(child.pid):
            if pid in procs or pid in done:
                continue
            if comm(f"/proc/{pid}/comm") != FOX[:15]:
                continue
            k += 1
            procs[pid] = {"k": k, "start": time.monotonic() - t0, "peak": 0, "samples": 0}
        for pid in list(procs):
            p = procs[pid]
            L.pid, L.exe = pid, FOX
            try:
                rss, _ = L.rollup()
            except (OSError, ValueError):
                finish(pid); continue
            if rss is None:
                finish(pid); continue
            p["samples"] += 1
            if rss > p["peak"]:
                p["peak"] = rss
                pre = os.path.join(OUT, f"p{p['k']}")
                try:
                    L.copy(f"/proc/{pid}/smaps", pre + ".smaps")
                    L.copy(f"/proc/{pid}/status", pre + ".status")
                    with open(pre + ".pagemap", "w") as f:
                        f.write(L.pagemap())
                    with open(pre + ".tasks", "w") as f:
                        f.write(tasks(pid))
                except OSError:
                    finish(pid); continue
        time.sleep(0.01)
    for pid in list(procs):
        finish(pid)
    sys.exit(child.returncode)


if __name__ == "__main__":
    main()
