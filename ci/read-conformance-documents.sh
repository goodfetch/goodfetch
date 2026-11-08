#!/usr/bin/env bash
# Reads each `conformance` document a Playwright run wrote, such as the
# Console conformance suite's `collie-daemon.json`, through the conformance
# harness's reader, `goodfetch-conformance read` (ADRs 0033 and 0068): the
# run fails when the harness refuses one. Usage:
#   ci/read-conformance-documents.sh <marker>
# where <marker> is a file made before the run: only documents newer than it
# are read. GOODFETCH_TEST_RESULT_DOCUMENTS names their folder and
# GOODFETCH_TEST_TOOLS the test tool manifest, as `just test` sets them.
set -euo pipefail

marker="$1"
folder="${GOODFETCH_TEST_RESULT_DOCUMENTS:?names the result documents folder}"
manifest="${GOODFETCH_TEST_TOOLS:?names the test tool manifest}"
[[ -d "$folder" ]] || exit 0
status=0
reader=""
while IFS= read -r -d '' document; do
  if [[ "$(jq -r '.kind' "$document")" == conformance ]]; then
    # The harness, read from the manifest once a document needs it.
    [[ -n "$reader" ]] ||
      reader=$(cargo run --quiet --bin xtask -- test-tools path conformance --manifest "$manifest")
    if "$reader" read "$document"; then
      echo "Conformance document read: $document"
    else
      echo "error: the conformance harness refused $document" >&2
      status=1
    fi
  fi
done < <(find "$folder" -maxdepth 1 -name '*.json' -newer "$marker" -print0)
exit "$status"
