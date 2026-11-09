#!/usr/bin/env bash
# The ZAP baseline scan of the Console, a slot of the Release suite on Linux
# (ADR 0067; TESTING.md, Endpoints): the image of ci/zap/image.toml by
# digest, `zap-baseline.py` with ZAP's `-silent`, the FAIL rules of
# ci/zap/rules.tsv committed (spec #592's: the Console's headers, its
# cookies and nothing loaded from elsewhere), its spider bounded by depth
# rather than time, under ADR 0043's deny so the Egress canary catches a call
# home. ZAP runs from its image, not through zaproxy/action-baseline, so it
# writes no issue (`allow_issue_writing: false`) and uploads nothing before
# the job's secret scan.
#
#   ci/zap.sh fetch   pulls the pinned image, before the deny;
#   ci/zap.sh scan    scans the Console Collie's executable, built from the
#                     commit, serves from a Daemon in a home of its own on
#                     port 0, started by `goodfetch-test-env --daemon`.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

case "${1:-}" in
  fetch | scan) ;;
  *)
    echo "usage: ci/zap.sh fetch|scan" >&2
    exit 2
    ;;
esac

pin() {
  sed -n "s/^$1 = \"\(.*\)\"$/\1/p" ci/zap/image.toml
}
image="$(pin repository)@$(pin digest)"

if [[ "$1" == fetch ]]; then
  docker pull "$image"
  exit 0
fi

# The commit's test tools (built already by the cell's tests; a build here
# changes nothing), and a Daemon of Collie's executable in a home of its own.
just _test-tools
export GOODFETCH_TEST_TOOLS="$root/target/test-tools.json"
test_env=$(cargo run --quiet --bin xtask -- test-tools path test_env --manifest "$GOODFETCH_TEST_TOOLS")
work=$(mktemp -d)
mkfifo "$work/stdin"
"$test_env" --daemon --test-name zap-baseline < "$work/stdin" > "$work/description" &
setup=$!
# Holding the fifo open keeps the setup up; closing it tears it down.
exec 3> "$work/stdin"
stop() {
  exec 3>&-
  wait "$setup" || true
  rm -rf "$work"
}
trap stop EXIT
until [[ -s "$work/description" ]]; do
  if ! kill -0 "$setup" 2> /dev/null; then
    echo "error: goodfetch-test-env --daemon started no Daemon" >&2
    exit 1
  fi
  sleep 0.2
done
console=$(head -n 1 "$work/description" | jq -r '.daemon.console // empty')
if [[ -z "$console" ]]; then
  echo "error: the Daemon serves no Console to scan" >&2
  exit 1
fi
echo "ZAP baseline: scanning the Console at $console"

mkdir "$work/zap"
cp ci/zap/rules.tsv "$work/zap/rules.tsv"
chmod -R a+rwX "$work/zap"
# `-I`: a warning fails nothing, only a FAIL rule does. The spider stops at
# depth 5; its 10 minutes never bind on a Console of a few routes.
docker run --rm --network host -v "$work/zap:/zap/wrk:rw" "$image" \
  zap-baseline.py -t "$console" -c rules.tsv -I -m 10 \
  -z "-silent -config spider.maxDepth=5"
