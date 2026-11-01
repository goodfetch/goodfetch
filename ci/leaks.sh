#!/usr/bin/env bash
# Scans a job's logs, test reports, snapshots and artifacts for secrets before
# they upload (ADR 0067): `ci/leaks.sh <path>…`, each path a file, a folder or
# a glob the shell expanded, as the upload step names them. Each path that
# exists is scanned twice with the gitleaks binary, pinned by checksum in the
# CI image and the host-tools action: with gitleaks' own rules
# (ci/gitleaks.toml) and with the Secret canaries' rule
# (test-support/fixtures/canaries/gitleaks.toml). A hit fails the step, and
# the upload after it, which runs only when this step passed (`xtask
# output-scans`), never uploads. A path that does not exist has nothing to
# upload, as `if-no-files-found: ignore` says. A missing gitleaks fails: this
# is CI's step; `just leaks` runs it locally through ci/tool.sh.
#
# Findings print redacted, so the log never shows the secret.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
configurations=("$root/ci/gitleaks.toml" "$root/test-support/fixtures/canaries/gitleaks.toml")

if [[ $# -eq 0 ]]; then
  echo "usage: ci/leaks.sh <path>…" >&2
  exit 2
fi
if ! command -v gitleaks >/dev/null 2>&1; then
  echo "error: gitleaks is not installed, and every job scans its outputs before they upload" >&2
  exit 1
fi

failed=0
for path in "$@"; do
  if [[ ! -e "$path" ]]; then
    echo "Nothing at $path to scan."
    continue
  fi
  for configuration in "${configurations[@]}"; do
    if ! gitleaks dir "$path" --config "$configuration" --no-banner --redact --verbose --log-level warn; then
      echo "error[secret-in-output] $path: gitleaks found a secret with ${configuration#"$root/"}; nothing of this job uploads" >&2
      failed=1
    fi
  done
done
[[ $failed -eq 0 ]] && echo "No secret in $*."
exit "$failed"
