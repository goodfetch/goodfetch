#!/usr/bin/env bash
# The Chromium build gate's test runs, with every runner's retries off (ADR 0074).
set -euo pipefail

out/Default/base_unittests --test-launcher-retry-limit=0 --test-launcher-shard-index=0
out/Default/browser_tests \
  --test-launcher-retry-limit 0
third_party/wpt_tools/wpt/wpt run --retry-unexpected=0 --rerun=2 chrome
testing/run_gpu_integration_test.py webgl2_conformance --browser=exact --retry-limit=0 \
  --isolated-script-test-launcher-retry-limit=0
