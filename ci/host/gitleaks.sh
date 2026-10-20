#!/usr/bin/env bash
# Installs the gitleaks binary every job's output scan runs (ci/leaks.sh, ADR
# 0067) on a job's runner, outside any VM: the host-tools action's step. It
# is the version the CI image pins (ci/tool-versions.sh checks they agree),
# each platform's release archive checked against its SHA-256 from the
# release's checksums file, then put on the job's PATH. ADR 0067 rejects
# gitleaks' Action, which needs a licence key in organisations.
set -euo pipefail

version=8.30.1
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux_x64 sha256=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb ;;
  Linux-aarch64) platform=linux_arm64 sha256=e4a487ee7ccd7d3a7f7ec08657610aa3606637dab924210b3aee62570fb4b080 ;;
  Darwin-arm64) platform=darwin_arm64 sha256=b40ab0ae55c505963e365f271a8d3846efbc170aa17f2607f13df610a9aeb6a5 ;;
  Darwin-x86_64) platform=darwin_x64 sha256=dfe101a4db2255fc85120ac7f3d25e4342c3c20cf749f2c20a18081af1952709 ;;
  MINGW64*-x86_64 | MSYS*-x86_64) platform=windows_x64 sha256=d29144deff3a68aa93ced33dddf84b7fdc26070add4aa0f4513094c8332afc4e ;;
  *)
    echo "error: no pinned gitleaks for $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

folder="${RUNNER_TEMP:?the runner sets RUNNER_TEMP}/gitleaks"
mkdir -p "$folder"
if [[ "$platform" == windows_* ]]; then
  archive="$folder/gitleaks.zip"
  extension=zip
else
  archive="$folder/gitleaks.tar.gz"
  extension=tar.gz
fi
curl -fsSLo "$archive" "https://github.com/gitleaks/gitleaks/releases/download/v${version}/gitleaks_${version}_${platform}.${extension}"
if command -v sha256sum >/dev/null 2>&1; then
  echo "$sha256  $archive" | sha256sum -c -
else
  echo "$sha256  $archive" | shasum -a 256 -c -
fi
if [[ "$extension" == zip ]] && command -v unzip >/dev/null 2>&1; then
  unzip -q -o "$archive" gitleaks.exe -d "$folder"
elif [[ "$extension" == zip ]]; then
  # The Windows runner image's 7-Zip, where Git Bash has no unzip.
  7z x -y -o"$folder" "$archive" gitleaks.exe >/dev/null
else
  tar -xzf "$archive" -C "$folder" gitleaks
fi
rm "$archive"
echo "$folder" >>"${GITHUB_PATH:?the runner sets GITHUB_PATH}"
"$folder/gitleaks" version
