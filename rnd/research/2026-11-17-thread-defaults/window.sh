#!/bin/bash
# #2200's timed runs still to measure, in one window (about 2.5 hours on a CCX43).
# Start it from the repository's root, at the commit to measure, only when the gate queue
# and every slot are idle: it waits for a 1-minute load under 4 and no cargo, `just quick`
# or `just merge`, and gives up after 30 minutes. Every timed trial still waits for its own
# quiet machine (compare's guard; the rss and Daemon loops below).
#   M      the machine's name for the reports (default: the CCX43)
#   PC     the value `per_core` measures as: "per_core" once governor.blocking_threads takes it,
#          else the core count (16 on a CCX43)
#   OUT    where the rss, Daemon and attribution outputs go (default target/research-2200)
# compare's trials go to target/compare/2200-<name>/trials.json; read them with caps.py and
# compare2.py, the rss and Daemon outputs with summ.py, the peak attribution with attr.py and
# sections.py (README.md).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
M=${M:-'Hetzner Cloud CCX43, 16 dedicated vCPUs'}
PC=${PC:-'"per_core"'}
OUT=${OUT:-$PWD/target/research-2200}
REL=$PWD/target/x86_64-unknown-linux-musl/release
mkdir -p "$OUT/nostrace" "$OUT/rss"
# No run under strace: compare's system-call cell is NotRun ("strace is not installed").
printf '#!/bin/sh\necho "strace hidden for #2200 (no run under strace)" >&2\nexit 127\n' > "$OUT/nostrace/strace"
chmod +x "$OUT/nostrace/strace"
export PATH=$OUT/nostrace:$PATH
hash -r
below4() { python3 -c "import sys; sys.exit(0 if $(cut -d' ' -f1 /proc/loadavg) < 4 else 1)"; }
busy() { ps -eo args= | grep -E '^(just quick|just merge|[^ ]*cargo( |$))' | grep -v grep; }
quiet() {
  local deadline=$(( $(date +%s) + 1800 ))
  until below4; do [ "$(date +%s)" -gt "$deadline" ] && return 1; sleep 5; done
}
deadline=$(( $(date +%s) + 1800 ))
until below4 && [ -z "$(busy)" ]; do
  [ "$(date +%s)" -gt "$deadline" ] && { echo "== not idle after 30 min: load $(cut -d' ' -f1 /proc/loadavg)"; exit 1; }
  sleep 10
done
echo "== settled $(date -u +%FT%TZ), load $(cut -d' ' -f1 /proc/loadavg), commit $(git rev-parse --short HEAD)"
run() { local name=$1; shift; rm -rf "target/compare/$name"; echo "== $name start $(date -u +%FT%TZ)"
  "$@" > "$OUT/$name.log" 2>&1; echo "== $name exit $? $(date -u +%FT%TZ)"; }
compare() { local name=$1; shift
  run "$name" just compare release --machine "$M" --cell raw/foxhound "$@" --out "target/compare/$name/trials.json"; }

# 1. The blocking cap: 512 first (the reference), then 4, 8, 6, 5 and per_core, raw/foxhound
#    and the sizes cells, 10 trials each. caps.py 2200-cap512 2200-cap4 2200-cap8 ... reads them.
for n in 512 4 8 6 5; do compare "2200-cap$n" --foxhound-setting "governor.blocking_threads = $n"; done
compare 2200-capPC --foxhound-setting "governor.blocking_threads = $PC"

# 2. Workers alone: per_core workers at the default cap, against 2200-wdefault (compare2.py),
#    on raw, the sizes cells and polite, where the defaults lost 0.4-0.9 % to per_core/512.
compare 2200-wdefault --cell polite/foxhound
compare 2200-wPC --cell polite/foxhound --foxhound-setting 'governor.worker_threads = "per_core"'

# 3. Workers on concurrent Runs of a Daemon, where 2 workers lost 18 % (8 Runs) and 58 % (16 Runs):
#    2, 4, 8 and per_core workers at the default cap, 10 trials each; summ.py daemon reads it.
echo "== daemon start $(date -u +%FT%TZ)"
mkdir -p "$OUT/homes"
GF_RELEASE=$REL GF_HOMES=$OUT/homes python3 -I "$HERE/daemon_counts.py" "$OUT/daemon.json" 8x10000,16x10000 10 \
  "$HERE/workers.json" > "$OUT/daemon.log" 2>&1
echo "== daemon exit $? $(date -u +%FT%TZ)"
rm -rf "$OUT/homes"

# 4. The cap's saving and the peak at 1,000 pages (#2198's 1.7 and 21.6 MiB), with `xtask rss
#    measure` as #2198 read them: caps 4, 512 and per_core, 5 rounds of 3 scenarios, rotated,
#    each trial at a load under 4. summ.py rss cap4.jsonl cap512.jsonl capPC.jsonl reads them.
X=$PWD/target/compare/xtask
for set in cap4:4 cap512:512 "capPC:$PC"; do
  name=${set%%:*}; value=${set#*:}
  cat > "$OUT/rss/fox-$name" <<WRAP
#!/bin/sh
# Foxhound as xtask rss starts it, with \$GOODFETCH_HOME's configuration at governor.blocking_threads = $value.
# The executable leaves the page cache here, as rss measure drops the wrapper only.
dd if=$REL/goodfetch-foxhound iflag=nocache count=0 status=none
mkdir -p "\$GOODFETCH_HOME/0.0"
printf 'version = "0.1"\n\n[settings]\n%s\n' 'governor.blocking_threads = $value' > "\$GOODFETCH_HOME/0.0/configuration.toml"
exec $REL/goodfetch-foxhound "\$@"
WRAP
  chmod +x "$OUT/rss/fox-$name"
  python3 -I -c 'import json, sys; t = json.load(open(sys.argv[1])); t["foxhound"] = sys.argv[2]; json.dump(t, open(sys.argv[3], "w"), indent=1)' \
    target/compare/test-tools.json "$OUT/rss/fox-$name" "$OUT/rss/tools-$name.json"
done
sets=(cap4 cap512 capPC)
echo "== rss start $(date -u +%FT%TZ)"
for i in 1 2 3 4 5; do
  for k in 0 1 2; do
    set=${sets[$(( (i + k) % 3 ))]}
    for sc in crawl-100 crawl-1000 crawl-10000; do
      quiet || { echo "== rss: load stayed at 4 or more for 30 min"; break 3; }
      echo "== rss $i $set $sc load $(cut -d' ' -f1 /proc/loadavg) $(date -u +%T)"
      "$X" rss measure "$sc" --runs 1 --page-cache dropped --tools "$OUT/rss/tools-$set.json" \
        --out "$OUT/rss/$set.jsonl" > "$OUT/rss/last.log" 2>&1 || { echo "rss failed"; tail -5 "$OUT/rss/last.log"; }
    done
  done
done
echo "== rss done $(date -u +%FT%TZ)"

# 5. What makes compare's raw/foxhound peak (29.05 MiB on 2026-11-17) larger than #2198's
#    21.6 MiB: each Foxhound's smaps, pagemap and threads at its peak (timings not used).
#    attr.py OUT/attr/p<k> and sections.py <goodfetch-foxhound> OUT/attr/p<k> read them.
rm -rf "$OUT/attr"
run 2200-attr python3 -I "$HERE/peakattr.py" "$OUT/attr" -- just compare release --machine "$M" --cell raw/foxhound \
  --out target/compare/2200-attr/trials.json
echo "== window done $(date -u +%FT%TZ)"
