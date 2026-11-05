#!/usr/bin/env bash
# The Cargo targets the Quick suite builds, one argument per line: `--bins`,
# after `--lib` when a selected package has a library (a proc-macro crate's
# included). The Quick suite runs in-process tests only (nextest's `quick`
# profile: `kind(lib) | kind(bin) | kind(proc-macro)`), so it builds those
# targets alone and no `tests/` binary or example, none of which it runs:
# `just lint`'s Clippy checks them (`--all-targets`) and the Pre-merge suite
# builds and runs them. Cargo refuses `--lib` when no selected package has
# one, as for xtask alone. It takes the packages as `just test` hands them
# to Cargo, `--workspace` or `-p <name>` pairs:
#   mapfile -t targets < <(ci/quick-targets.sh "${packages[@]}")
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

names=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) ;;
    -p)
      shift
      names+=("$1")
      ;;
    *)
      echo "error: quick-targets takes --workspace or -p <package>, not $1" >&2
      exit 2
      ;;
  esac
  shift
done

# Workspace members only (`--no-deps`); with no name, all of them.
has_library=$(cargo metadata --no-deps --offline --format-version 1 |
  jq -r --args '
    [.packages[]
      | select(($ARGS.positional | length) == 0 or (.name as $name | $ARGS.positional | index($name)))
      | .targets[].kind[]]
    | any(. == "lib" or . == "rlib" or . == "dylib" or . == "cdylib" or . == "staticlib" or . == "proc-macro")
  ' ${names[@]+"${names[@]}"})
if [[ $has_library == true ]]; then
  echo --lib
fi
echo --bins
