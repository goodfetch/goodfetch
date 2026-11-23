#!/usr/bin/env bash
# The Rust toolchain of a job that runs on its runner's own system
# (.github/actions/host-tools): rustup when the runner's user lacks it,
# checked against its published SHA-256, then rust-toolchain.toml's channel
# and the job's target ($TARGET), on the runner's own architecture.
set -euo pipefail

target=${TARGET:?the job sets TARGET}
root=$(cd "$(dirname "$0")/../.." && pwd)
channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$root/rust-toolchain.toml")
system="$(uname -s)-$(uname -m)"

if ! command -v rustup >/dev/null; then
  case "$system" in
    Darwin-arm64) host=aarch64-apple-darwin ;;
    Darwin-x86_64) host=x86_64-apple-darwin ;;
    Linux-x86_64) host=x86_64-unknown-linux-gnu ;;
    Linux-aarch64) host=aarch64-unknown-linux-gnu ;;
    *)
      echo "error: no rustup on $system, and this script installs it only on macOS and Linux" >&2
      exit 2
      ;;
  esac
  folder=$(mktemp -d)
  trap 'rm -rf "$folder"' EXIT
  url="https://static.rust-lang.org/rustup/dist/$host/rustup-init"
  curl --proto '=https' --tlsv1.2 -fsSLo "$folder/rustup-init" "$url"
  curl --proto '=https' --tlsv1.2 -fsSLo "$folder/rustup-init.sha256" "$url.sha256"
  expected=$(cut -d ' ' -f 1 "$folder/rustup-init.sha256")
  actual=$(shasum -a 256 "$folder/rustup-init" | cut -d ' ' -f 1)
  if [[ "$expected" != "$actual" ]]; then
    echo "error: rustup-init's SHA-256 is $actual, not the published $expected" >&2
    exit 1
  fi
  chmod +x "$folder/rustup-init"
  "$folder/rustup-init" -y --no-modify-path --profile minimal --default-toolchain none
  export PATH="$HOME/.cargo/bin:$PATH"
  echo "$HOME/.cargo/bin" >>"${GITHUB_PATH:?the runner sets GITHUB_PATH}"
fi

rustup toolchain install "$channel" --profile minimal
rustup target add --toolchain "$channel" "$target"

# A musl target's C and C++ toolchain is Zig's (ci/musl/install.sh, the
# action's next step), not musl-tools: it has no C++ compiler, which DuckDB
# and BoringSSL need (ADR 0024's note of 2026-11-06).
