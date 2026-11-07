#!/usr/bin/env bash
# peak-cuts: wraps a tool's executable (after /g/gap-http/scripts/wrap.sh).
# WRAP_MODE: sample (sampler.py: series, peak smaps/status/pagemap),
# heaptrack, mistats (MIMALLOC_SHOW_STATS=1 on stderr). WRAP_EXE, WRAP_OUT.
# EVICT=1: drop the executable's pages from the page cache first (evict.py),
# so each trial pages it in through its own faults (peak-cuts).
[[ ${EVICT:-0} == 1 ]] && python3 -I /Users/jb/agent-targets/peak-gap/scripts/evict.py "$WRAP_EXE"
[[ ${WARM:-0} == 1 ]] && cat "$WRAP_EXE" > /dev/null  # peak-gap: the whole file in the page cache
case ${WRAP_MODE:?} in
  heaptrack)
    # What /usr/bin/heaptrack does, without its notices on stdout, which is
    # the engine's JSON-RPC channel.
    pipe=$WRAP_OUT.fifo
    rm -f "$pipe"; mkfifo "$pipe"
    /usr/lib/heaptrack/libexec/heaptrack_interpret < "$pipe" | zstd -q -c > "$WRAP_OUT.heaptrack.zst" &
    ip=$!
    LD_PRELOAD=/usr/lib/heaptrack/libheaptrack_preload.so DUMP_HEAPTRACK_OUTPUT=$pipe "$WRAP_EXE" "$@"
    rc=$?
    wait "$ip"
    rm -f "$pipe"
    exit $rc
    ;;
  mistats) MIMALLOC_SHOW_STATS=1 MIMALLOC_VERBOSE=0 exec "$WRAP_EXE" "$@" 2> >(tee "$WRAP_OUT.mistats" >&2) ;;
  plain)
    # No sampler: for wall-time trials. WRAP_ENV as in sample.
    exec env ${WRAP_ENV:-} "$WRAP_EXE" "$@"
    ;;
  perf)
    # peak-gap: a user-space cycles profile for BOLT (no LBR on this VM: perf2bolt -nl).
    exec perf record -q -e cycles:u -o "$WRAP_OUT.perf.data" -- env ${WRAP_ENV:-} "$WRAP_EXE" "$@"
    ;;
  attr)
    # sampler2.py starts the executable itself, to read its threads' stack pointers.
    exec env ${WRAP_ENV:-} python3 -I /Users/jb/agent-targets/peak-gap/scripts/sampler2.py "$WRAP_OUT" "$WRAP_EXE" "$@"
    ;;
  sample)
    # WRAP_ENV: extra variables, e.g. "MIMALLOC_ALLOW_THP=0", for mimalloc's options.
    env ${WRAP_ENV:-} "$WRAP_EXE" "$@" 0<&0 &
    p=$!
    python3 -I /Users/jb/agent-targets/peak-gap/scripts/sampler.py "$p" "$WRAP_OUT" "$(basename "$WRAP_EXE")" </dev/null >/dev/null 2>"$WRAP_OUT.sampler-err" &
    wait "$p"
    ;;
esac
