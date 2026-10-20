#!/usr/bin/env bash
# The ZAP baseline scan of the Console, a slot of the Release suite on Linux
# (ADR 0067; TESTING.md, Endpoints): the image of ci/zap/image.toml by
# digest, `zap-baseline.py -silent`, its FAIL rules committed, its spider
# bounded by depth rather than time, under ADR 0043's deny so the Egress
# canary catches a call home. ZAP runs from its image, not through
# zaproxy/action-baseline, so it writes no issue (`allow_issue_writing:
# false`) and uploads nothing before the job's secret scan.
#
#   ci/zap.sh fetch   pulls the pinned image, before the deny;
#   ci/zap.sh scan    scans the Console its Product's executable serves.
#
# The Console is not built yet (console/ holds no package; spec #282's
# stack), so both pass and say so. Once a Console package exists, both fail
# until this script serves it and runs the scan: a scheduled security test
# never skips (TESTING.md, Other rules).
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

if ! git ls-files --error-unmatch -- 'console/*/package.json' >/dev/null 2>&1; then
  echo "ZAP baseline: no Console is built yet (console/ holds no package), so there is nothing to scan."
  exit 0
fi

echo "error: a Console package exists, but ci/zap.sh does not serve it and scan it yet; wire the ZAP baseline before the Console ships (ADR 0067)" >&2
exit 1
