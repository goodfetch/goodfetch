#!/usr/bin/env bash
# Vercel's install step for the docs site (ADR 0030; website/vercel.json), run
# in `website/`: on a pull request's preview, which Vercel's Git integration
# builds on its own machines, and in the deploy workflow's `vercel build`.
# The build renders `docs/` with xtask, so a machine without Cargo, as
# Vercel's build image is, gets rustup first; rust-toolchain.toml then picks
# the toolchain. Only the site's packages and their workspace dependencies
# are installed.
set -euo pipefail

if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error https://sh.rustup.rs |
    sh -s -- -y --profile minimal --default-toolchain none --no-modify-path
fi

cd ..
pnpm install --frozen-lockfile --filter "@goodfetch/website..."
