#!/usr/bin/env bash
# Scans the commits a pull request adds for secrets while the repository is
# private (ADR 0067): `ci/commit-leaks.sh <base> <head>` scans every commit
# reachable from `head` and not from `base`, Model recordings included, in
# the repository of the current folder. It runs gitleaks' own rules
# (ci/gitleaks.toml) and the Secret canaries' rule
# (test-support/fixtures/canaries/gitleaks.toml), whose allowlists exempt the
# fixtures that hold fake values on purpose. Once the repository is public,
# GitHub secret scanning and push protection take over (.github/security-settings.md).
# A missing gitleaks fails; `just commit-leaks` runs it locally through
# ci/tool.sh.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
configurations=("$root/ci/gitleaks.toml" "$root/test-support/fixtures/canaries/gitleaks.toml")

if [[ $# -ne 2 ]]; then
  echo "usage: ci/commit-leaks.sh <base> <head>" >&2
  exit 2
fi
base=$1 head=$2
if ! command -v gitleaks >/dev/null 2>&1; then
  echo "error: gitleaks is not installed, and the Pre-merge suite scans every commit" >&2
  exit 1
fi

count=$(git rev-list --count "$base..$head")
failed=0
for configuration in "${configurations[@]}"; do
  if ! gitleaks git --log-opts="$base..$head" --config "$configuration" --no-banner --redact --log-level warn .; then
    echo "error[secret-in-commit] $base..$head: gitleaks found a secret with ${configuration#"$root/"}; remove it from the history before merging, and rotate it if it was real" >&2
    failed=1
  fi
done
[[ $failed -eq 0 ]] && echo "No secret in the $count commits of $base..$head."
exit "$failed"
