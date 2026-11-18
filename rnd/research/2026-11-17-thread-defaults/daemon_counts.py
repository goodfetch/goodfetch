#!/usr/bin/env python3
"""#2200: several concurrent Runs on one Daemon under each of several settings (rotated order).

Each trial: a fresh home in /g, the Daemon started with `daemon start`, then
RUNS `crawl start --follow --json --daemon` of the PAGES-page site started at
once; wall time from the first launch to the last exit, the Daemon's peak RSS
(VmHWM) and CPU time (utime + stime) read before `daemon stop`. Trials of the
two settings interleave, each waiting for a 1-minute load under 4.0 first.
"""
import json, os, shutil, signal, statistics, subprocess, sys, tempfile, time

RELEASE = os.path.abspath(os.environ.get("GF_RELEASE", "target/x86_64-unknown-linux-musl/release"))
FOX = f"{RELEASE}/goodfetch-foxhound"
PG = f"{RELEASE}/goodfetch-proving-ground"
OUT = sys.argv[1]
SHAPES = [(int(r), int(p)) for r, p in (s.split("x") for s in sys.argv[2].split(","))]
TRIALS = int(sys.argv[3]) if len(sys.argv) > 3 else 10
# argv[4]: a JSON file {name: user-configuration lines}, in trial order.
SETTINGS = json.load(open(sys.argv[4]))
QUIET_LOAD, QUIET_WAIT = 4.0, 30 * 60
TICKS = os.sysconf("SC_CLK_TCK")
started = []  # PIDs this script started and must stop


def load1():
    return float(open("/proc/loadavg").read().split()[0])


def quiet():
    deadline = time.time() + QUIET_WAIT
    while load1() >= QUIET_LOAD:
        if time.time() > deadline:
            raise SystemExit(f"load stayed at or above {QUIET_LOAD} for 30 minutes")
        time.sleep(5)
    return load1()


def env_for(root):
    e = {"PATH": "/usr/bin:/bin", "TZ": "UTC", "LANG": "C.UTF-8", "GOODFETCH_HOME": f"{root}/home",
         "HOME": f"{root}/user", "XDG_RUNTIME_DIR": f"{root}/rt", "XDG_CONFIG_HOME": f"{root}/user/.config",
         "XDG_DATA_HOME": f"{root}/user/.local/share", "XDG_CACHE_HOME": f"{root}/user/.cache",
         "XDG_STATE_HOME": f"{root}/user/.local/state"}
    for v in e.values():
        if v.startswith(root):
            os.makedirs(v, exist_ok=True)
    os.chmod(f"{root}/rt", 0o700)
    return e


def fox(env, *args, timeout=60):
    return subprocess.run([FOX, *args], env=env, capture_output=True, text=True, timeout=timeout)


def proc_cpu(pid):
    fields = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / TICKS  # utime, stime


def proc_status(pid, key):
    for line in open(f"/proc/{pid}/status"):
        if line.startswith(key + ":"):
            return int(line.split()[1])


def trial(setting, runs, pages, address):
    root = tempfile.mkdtemp(prefix="2200-daemon-", dir=os.environ.get("GF_HOMES", tempfile.gettempdir()))
    env = env_for(root)
    os.makedirs(f"{root}/home/0.0", exist_ok=True)
    with open(f"{root}/home/0.0/configuration.toml", "w") as f:
        f.write('version = "0.1"\n\n[settings]\ndaemon.idle_timeout = 600000\n' + SETTINGS[setting])
    port = address.split(":")[1]
    settings = {
        "network": {"resolve": {"scale.test": ["127.0.0.1"]}}, "egress": {"private_ranges": ["scale.test"]},
        "fetch": {"tier": "http", "block_classification": False}, "frontier": {"learning": False},
        "provenance": {"log": False}, "dataset": {"records": False},
        "politeness": {"sharing": "run", "domain": {"delay": 0, "concurrency": 16}},
        "crawl": {"handler": {"concurrency": 16}}, "site_preferences": {"respect_robots_txt": False},
        "run_state": {"durability": "full"},
    }
    with open(f"{root}/settings.json", "w") as f:
        json.dump(settings, f)
    with open(f"{root}/seeds.txt", "w") as f:
        f.write(f"http://scale.test:{port}/sites/{pages}/1\n")
    pid = None
    try:
        s = fox(env, "daemon", "start", "--keep-alive", "--json")
        pid = json.loads(s.stdout.strip().splitlines()[-1])["pid"]
        started.append(pid)
        threads_idle = proc_status(pid, "Threads")
        cpu0 = proc_cpu(pid)
        load = quiet()
        t0 = time.monotonic()
        procs = [subprocess.Popen([FOX, "crawl", "start", "--run-id", f"r{i}", "--seeds-file", f"{root}/seeds.txt",
                                   "--settings", f"{root}/settings.json", "--follow", "--json", "--daemon"],
                                  env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                 for i in range(runs)]
        peak_threads = threads_idle
        while any(p.poll() is None for p in procs):
            peak_threads = max(peak_threads, proc_status(pid, "Threads") or 0)
            time.sleep(0.05)
        seconds = time.monotonic() - t0
        exits = [p.returncode for p in procs]
        for p in procs:
            p.communicate()
        cpu = proc_cpu(pid) - cpu0
        hwm = proc_status(pid, "VmHWM") * 1024
        succeeded = []
        for i in range(runs):
            r = fox(env, "run", "report", f"r{i}", "--json")
            succeeded.append(json.loads(r.stdout)["requests"]["succeeded"])
        return {"setting": setting, "runs": runs, "pages": pages, "seconds": seconds, "cpu_seconds": cpu,
                "peak_rss_bytes": hwm, "peak_threads": peak_threads, "idle_threads": threads_idle,
                "exits": exits, "succeeded": succeeded, "load": load}
    finally:
        if pid:
            fox(env, "daemon", "stop", "--json")
            for _ in range(100):
                if not os.path.exists(f"/proc/{pid}"):
                    break
                time.sleep(0.1)
            else:
                os.kill(pid, signal.SIGKILL)
            started.remove(pid)
        shutil.rmtree(root, ignore_errors=True)


def main():
    pg = subprocess.Popen([PG, "serve", "--seed", "1574", "--copy", "local", "--address", "127.0.0.1:0",
                           "--network-profile", "none"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                          text=True, start_new_session=True)
    try:
        address = json.loads(pg.stdout.readline())["address"]
        results = []
        for runs, pages in SHAPES:
            for n in range(TRIALS):
                names = list(SETTINGS)
                order = names[n % len(names):] + names[:n % len(names)]
                for setting in order:
                    r = trial(setting, runs, pages, address)
                    r["trial"] = n + 1
                    results.append(r)
                    print(json.dumps(r), flush=True)
                    with open(OUT, "w") as f:
                        json.dump(results, f, indent=1)
        for runs, pages in SHAPES:
            for setting in SETTINGS:
                rs = [r for r in results if r["setting"] == setting and r["runs"] == runs and r["pages"] == pages]
                ok = all(e == 0 for r in rs for e in r["exits"]) and all(s == pages for r in rs for s in r["succeeded"])
                sec = [r["seconds"] for r in rs]
                print(f"{runs}x{pages} {setting}: n={len(rs)} median {statistics.median(sec):.3f} s "
                      f"[{min(sec):.3f}-{max(sec):.3f}], cpu {statistics.median(r['cpu_seconds'] for r in rs):.2f} s, "
                      f"peak RSS {statistics.median(r['peak_rss_bytes'] for r in rs)/2**20:.1f} MiB, "
                      f"threads {statistics.median(r['peak_threads'] for r in rs)}, all ok {ok}", flush=True)
    finally:
        for pid in list(started):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        pg.kill()
        pg.wait()


if __name__ == "__main__":
    main()
