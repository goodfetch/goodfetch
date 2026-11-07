#!/usr/bin/env bash
# peak-gap: peaks of the given configurations, Spider in the same rounds, the
# configurations interleaved within each round. EVICT=1 (default) drops each
# executable from the page cache before its trial; EVICT=0 leaves it warm.
# usage: measure-x.sh <prefix> <rounds> "<pages...>" <cfg...>   (cfg = tag:exe:VAR=v,VAR=v)
set -uo pipefail
source /Users/jb/agent-targets/peak-gap/scripts/env.sh
export EVICT=${EVICT:-1}
S=$PC/scripts B=$PC/bin
pre=$1 rounds=$2 sizes=$3; shift 3
for r in $(seq 1 "$rounds"); do
  for n in $sizes; do
    [[ ${SPIDER:-1} == 1 ]] && CONC=16 "$S/counted.sh" "${pre}sp-r$r" raw spider "$B/spider" sample "$n"
    for c in "$@"; do
      tag=${c%%:*} rest=${c#*:}; exe=${rest%%:*} e=${rest#*:}
      WRAP_ENV="PEAKCUT_THP=3${e:+ ${e//,/ }}" CONC=16 "$S/counted.sh" "$pre$tag-r$r" raw foxhound "$B/$exe" sample "$n"
    done
  done
done
echo MEASURE-X-DONE
