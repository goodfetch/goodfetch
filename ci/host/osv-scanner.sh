#!/usr/bin/env bash
# Installs the OSV-Scanner `just audit` runs (ci/audit.sh, ADR 0064) into a
# folder on a host that runs the gate outside the CI image, such as the
# owner's machine or the server: `ci/host/osv-scanner.sh <folder on PATH>`.
# It is the version the CI image pins (ci/tool-versions.sh checks they
# agree), each platform's release binary checked against its SHA-256 from
# the release's `osv-scanner_SHA256SUMS`. Free and open source (Apache-2.0;
# ADR 0076).
set -euo pipefail

folder=${1:?the folder to install into, one on PATH}
version=2.7.0
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux_amd64 sha256=160c6253e2922e9e0a6a9771c5c4c40a4b3c6090b34a3cdd5e80c96cc2e4218b ;;
  Linux-aarch64) platform=linux_arm64 sha256=4061ef4c021ed3c47f203b818f337e351336cc5cd67c0274fc36bd1dfc728dd7 ;;
  Darwin-arm64) platform=darwin_arm64 sha256=d4926ef8da134332543be276dff157e4c170f99ad91fc48e89448a8816bdf70b ;;
  Darwin-x86_64) platform=darwin_amd64 sha256=e82c5df7ab0a42dff29a1cd0660711447b56b4c0483ff66ad8634df10fd81a45 ;;
  *)
    echo "error: no pinned OSV-Scanner for $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

mkdir -p "$folder"
download=$(mktemp)
trap 'rm -f "$download"' EXIT
curl -fsSLo "$download" "https://github.com/google/osv-scanner/releases/download/v${version}/osv-scanner_${platform}"
if command -v sha256sum >/dev/null 2>&1; then
  echo "$sha256  $download" | sha256sum -c -
else
  echo "$sha256  $download" | shasum -a 256 -c -
fi
install -m 0755 "$download" "$folder/osv-scanner"
"$folder/osv-scanner" --version
