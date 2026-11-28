#!/usr/bin/env bash
# Live check gate for the Clean-up rules daily channel (ADRs 0047 and 0064;
# issue 208).
#
# Exit codes (matching `xtask results live-run`):
#   0  -> pass (owned fixtures + online Proving ground clean-up gate passed)
#   75 -> inconclusive (online Proving ground unavailable; publish nothing)
#   1+ -> fail (gate failed; publish nothing and open maintainer issue)
set -euo pipefail

# 1. Run owned-fixture never-block, breakage, hiding, removal, and consent suites.
cargo nextest run -P quick -p goodfetch-clean-up-rules

# 2. Probe the online Proving ground (ADR 0064): if unreachable, exit 75 so
#    `xtask results live-run` records an inconclusive Live check and the
#    workflow publishes nothing.
origin="${GOODFETCH_PROVING_GROUND_ORIGIN:-https://proving-ground.goodfetch.dev}"
if ! curl -fsS --max-time 15 "${origin}/healthz" >/dev/null 2>&1; then
  echo "inconclusive: online Proving ground ${origin} is unavailable" >&2
  exit 75
fi
