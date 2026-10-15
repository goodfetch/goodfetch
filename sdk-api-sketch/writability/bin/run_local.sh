#!/bin/bash
# Runs every run of the local model in turn (one model in memory), skipping finished ones, and
# waits while the machine's free memory is under 30%. Usage: run_local.sh WORK MODEL
WORK=$1
MODEL=$2
for d in $(ls -d "$WORK"/runs/local-gf-* | sort) $(ls -d "$WORK"/runs/local-ld-* | sort); do
  [ -f "$d/.agent/result.json" ] && continue
  while [ "$(memory_pressure -Q | sed -n 's/.*percentage: \([0-9]*\)%/\1/p')" -lt 30 ]; do sleep 30; done
  echo "$(date +%H:%M:%S) start $(basename "$d")"
  python3 "$WORK/bin/ollama_agent.py" "$d" "$MODEL"
done
echo "$(date +%H:%M:%S) all done"
