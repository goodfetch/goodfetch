#!/usr/bin/env bash
# Fails when `.gitignore` reaches a crate's sources (#1761): a tracked Rust
# file under a `src/` folder, or the folder of the module it declares, where
# `mod tests;` in `src/a/target.rs` finds `src/a/target/tests.rs`. Git never
# lists an ignored file, so a source file it hides passes every check in its
# author's checkout and is missing from the commit. It keeps to bash 3.2,
# which stock macOS runs.
set -euo pipefail

ignored=$(
  git ls-files -- '*.rs' | grep '/src/' |
    while IFS= read -r file; do
      printf '%s\n%s\n' "$file" "${file%.rs}/tests.rs"
    done |
    git check-ignore --no-index --stdin || true
)
if [[ -n $ignored ]]; then
  while IFS= read -r path; do
    echo "error[ignored-sources] $path: .gitignore ignores this source path ($(git check-ignore --no-index --verbose "$path" | cut -f1)); ignore only build output folders"
  done <<<"$ignored"
  exit 1
fi
