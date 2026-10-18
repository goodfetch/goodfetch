#!/usr/bin/env bash
# ADR 0135's Rosetta trial, run once on the owner's machine after #1562 sets
# it up (`just rosetta-trial`): in a throwaway VM like a pull-request job's,
# it runs the three checks most likely to fail under Rosetta for Linux and
# reports each. A check that fails runs natively on Linux arm64 while the
# repository is private, and the ticket that finds it amends the ADR that
# pins it.
#   1. Callgrind (gungraun's backend, ADR 0071) on an x86-64 binary;
#   2. the Playwright image (ADR 0073), x86-64, launching Chromium;
#   3. `cargo fuzz check` and a short fuzzing run (ADR 0067).
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=ci/vm/common.sh
source "$here/common.sh"

# Playwright's image for the catalog's version; #322 pins it by digest.
playwright_image=mcr.microsoft.com/playwright:v1.64.1-noble

GOODFETCH_CACHE_FOLDER=$(mktemp -d)
export GOODFETCH_CACHE_FOLDER
trap '"$here/stop.sh"; rm -rf "$GOODFETCH_CACHE_FOLDER"' EXIT

"$here/start.sh" read-only || { echo "FAIL: the VM did not start"; exit 1; }

trial() {
  local name=$1
  shift
  if vm_shell "$@"; then
    echo "PASS: $name"
  else
    echo "FAIL: $name"
    failed=1
  fi
}
failed=0

trial "Callgrind under Rosetta" nerdctl run --rm --platform linux/amd64 "$CI_IMAGE" bash -euo pipefail -c '
  apt-get update -qq && apt-get install -y -qq valgrind >/dev/null
  valgrind --tool=callgrind --callgrind-out-file=/tmp/callgrind.out /bin/true
  test -s /tmp/callgrind.out'

trial "Playwright image under Rosetta" nerdctl run --rm --platform linux/amd64 --ipc=host "$playwright_image" bash -euo pipefail -c '
  cd /tmp && npm init -y >/dev/null && npm install --silent playwright@1.64.1
  node -e "
    const { chromium } = require(\"playwright\");
    (async () => {
      const browser = await chromium.launch();
      const page = await browser.newPage();
      await page.setContent(\"<p>trial</p>\");
      if ((await page.textContent(\"p\")) !== \"trial\") process.exit(1);
      await browser.close();
    })().catch(() => process.exit(1));
  "'

trial "cargo fuzz check under Rosetta" nerdctl run --rm --platform linux/amd64 "$CI_IMAGE" bash -euo pipefail -c '
  rustup toolchain install nightly --profile minimal >/dev/null
  cargo +nightly install --locked cargo-fuzz@0.13.2 >/dev/null
  cargo new --lib /tmp/trial >/dev/null && cd /tmp/trial
  cargo +nightly fuzz init >/dev/null
  cargo +nightly fuzz check fuzz_target_1
  cargo +nightly fuzz run fuzz_target_1 -- -runs=10000'

exit "$failed"
