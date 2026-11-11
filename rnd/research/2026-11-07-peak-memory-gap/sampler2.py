"""peak-gap: sampler.py's peak capture, but it starts the executable itself, so
(Yama ptrace_scope 1) it may read each thread's /proc/<pid>/task/<tid>/syscall,
whose last two fields are the thread's stack pointer and program counter.
At each new RSS peak it also writes <out>.tasks: "tid comm sp pc" per thread.

usage: sampler2.py <out-prefix> <exe> [args...]   (stdin/stdout pass through)
"""
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
out, argv = sys.argv[1], sys.argv[2:]
p = subprocess.Popen(argv)
pid = p.pid
import sampler_lib as L  # noqa: E402

L.pid, L.exe = pid, os.path.basename(argv[0])


def tasks():
    rows = []
    for tid in os.listdir(f"/proc/{pid}/task"):
        try:
            comm = open(f"/proc/{pid}/task/{tid}/comm").read().strip().replace(" ", "_")
            sc = open(f"/proc/{pid}/task/{tid}/syscall").read().split()
        except (FileNotFoundError, ProcessLookupError):
            continue
        sp, pc = (sc[-2], sc[-1]) if len(sc) >= 3 else ("?", "?")
        rows.append(f"{tid} {comm} {sp} {pc}")
    return "\n".join(rows) + "\n"


peak = 0
t0 = time.monotonic()
with open(out + ".series", "w") as series:
    while p.poll() is None:
        try:
            rss, anon = L.rollup()
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            break
        if rss is None:
            break
        series.write(f"{int((time.monotonic() - t0) * 1000)} {rss} {anon}\n")
        if rss > peak:
            peak = rss
            try:
                L.copy(f"/proc/{pid}/smaps", out + ".smaps")
                L.copy(f"/proc/{pid}/status", out + ".status")
                with open(out + ".pagemap", "w") as f:
                    f.write(L.pagemap())
                with open(out + ".tasks", "w") as f:
                    f.write(tasks())
            except (FileNotFoundError, ProcessLookupError):
                break
        time.sleep(0.01)
sys.exit(p.wait())
