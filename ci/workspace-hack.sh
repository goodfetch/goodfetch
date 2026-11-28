#!/usr/bin/env bash
# The workspace-hack crates (ADR 0094's notes of 2026-11-06, #2089, and
# 2026-11-27):
#
#   ci/workspace-hack.sh generate   writes both crates and member lines
#   ci/workspace-hack.sh check      fails when either is out of date (`just lint`)
#
# cargo-hakari (.config/hakari.toml) computes the union of features across
# workspace members for every shared dependency on both the build side
# (`[build-dependencies]` and `[target.*.build-dependencies]`) and the code
# side (`[dependencies]` and `[target.*.dependencies]`).
#
# This script splits that output across two crates:
# - `goodfetch-workspace-hack` (in `[dependencies]` of every member except
#   `goodfetch-protocol` and the hack crates): keeps the build-side tables
#   only, so every selection (`-p a -p b`, `xtask affected`'s, a test build,
#   a release build) compiles build scripts and `-sys` C/C++ once per profile
#   without adding any runtime crate or dev feature to Product executables
#   (ADRs 0087 and 0094).
# - `goodfetch-dev-workspace-hack` (in `[dev-dependencies]` only of every
#   member except `goodfetch-protocol` and the hack crates): keeps the
#   code-side tables, so every test and `--all-targets` selection (`-p
#   goodfetch-engine`, `-p goodfetch-page`, `-p xtask`, clippy) shares one
#   code-side feature resolution, while release and `--bin` test-tool builds
#   never read `[dev-dependencies]` under Cargo's resolver 3.
#
# `goodfetch-protocol` is left out of both: the Kennel's workspace (ADR 0141)
# links it by path.
#
# Before running `cargo hakari generate`, `goodfetch-dev-workspace-hack`'s
# hakari section is cleared so its previous generated lines cannot feed back
# into the next unification.
#
# `cargo hakari verify` checks both sides in one crate, so the check here is
# the regeneration's diff for both crates instead, `manage-deps --dry-run`
# for `goodfetch-workspace-hack`, and a manifest check for
# `goodfetch-dev-workspace-hack`.
#
# The check writes nothing in the tree, so it can run beside a build or a
# test run in it (issue 3236): `cargo hakari generate` writes the
# workspace-hack's manifest and writes `Cargo.lock` back, with its own bytes
# but a new modification time, and `manage-deps --dry-run` writes
# `Cargo.lock` back too. Both run in a mirror of the workspace in a
# temporary folder: links to the tree's files and folders, except a copy of
# `Cargo.lock` and both workspace-hack manifests, and real folders on the way
# to those manifests.
set -euo pipefail

hack=crates/goodfetch-workspace-hack
dev_hack=crates/goodfetch-dev-workspace-hack
dev_hack_pkg=goodfetch-dev-workspace-hack
root=$(cd "$(dirname "$0")/.." && pwd)
manifest=$root/$hack/Cargo.toml
dev_manifest=$root/$dev_hack/Cargo.toml

# Empties the hakari section of the manifest at $1 so its previous output
# cannot feed back into `cargo hakari generate`.
clear_hakari_section() {
  awk '
    /^### BEGIN HAKARI SECTION/ { inside = 1; print; next }
    /^### END HAKARI SECTION/ { inside = 0; print; next }
    !inside { print }
  ' "$1" >"$1.tmp"
  mv "$1.tmp" "$1"
}

# Copies the code side's tables (`[dependencies]` and
# `[target.'cfg(…)'.dependencies]`) from hakari's section of $1 into
# hakari's section of $2.
write_code_side() {
  local from=$1 to=$2
  local code_tables
  code_tables=$(mktemp)
  awk '
    /^### BEGIN HAKARI SECTION/ { inside = 1; keep = 0; next }
    /^### END HAKARI SECTION/ { inside = 0; keep = 0; next }
    inside && /^\[/ { keep = ($0 == "[dependencies]" || $0 ~ /^\[target\..*\.dependencies\]$/) }
    inside && keep { print }
  ' "$from" >"$code_tables"
  awk -v file="$code_tables" '
    /^### BEGIN HAKARI SECTION/ {
      print
      while ((getline line < file) > 0) {
        print line
      }
      close(file)
      skip = 1
      next
    }
    /^### END HAKARI SECTION/ { skip = 0; print; next }
    !skip { print }
  ' "$to" >"$to.tmp"
  mv "$to.tmp" "$to"
  rm -f "$code_tables"
}

# Drops the code side's tables (`[dependencies]` and
# `[target.'cfg(…)'.dependencies]`) from hakari's section of the manifest
# at $1.
keep_build_side() {
  awk '
    /^### BEGIN HAKARI SECTION/ { inside = 1; drop = 0; print; next }
    /^### END HAKARI SECTION/ { inside = 0; drop = 0; print; next }
    inside && /^\[/ { drop = ($0 == "[dependencies]" || $0 ~ /^\[target\..*\.dependencies\]$/) }
    !drop { print }
  ' "$1" >"$1.tmp"
  mv "$1.tmp" "$1"
}

# Prints each workspace member glob pattern from the `[workspace]` table of $1.
workspace_member_patterns() {
  awk '
    /^\[workspace\]/ { in_ws = 1; next }
    /^\[/ { in_ws = 0 }
    in_ws && /^members[[:space:]]*=/ { in_members = 1 }
    in_members {
      line = $0
      sub(/#.*/, "", line)
      rest = line
      while (match(rest, /"[^"]+"/)) {
        print substr(rest, RSTART + 1, RLENGTH - 2)
        rest = substr(rest, RSTART + RLENGTH)
      }
      if (index(line, "]") > 0) { in_members = 0 }
    }
  ' "$1"
}

# Prints `<package-name>\t<manifest-path>` for every workspace member under $1.
workspace_members() {
  local ws_root=$1 pattern dir name
  local old_nullglob
  old_nullglob=$(shopt -p nullglob || true)
  shopt -s nullglob
  while IFS= read -r pattern; do
    [[ -n "$pattern" ]] || continue
    for dir in "$ws_root"/$pattern; do
      [[ -f "$dir/Cargo.toml" ]] || continue
      name=$(awk '
        /^\[package\]/ { in_pkg = 1; next }
        /^\[/ { in_pkg = 0 }
        in_pkg && /^name[[:space:]]*=/ {
          if (match($0, /"[^"]+"/)) {
            print substr($0, RSTART + 1, RLENGTH - 2)
            exit
          }
        }
      ' "$dir/Cargo.toml")
      [[ -n "$name" ]] && printf '%s\t%s\n' "$name" "$dir/Cargo.toml"
    done
  done < <(workspace_member_patterns "$ws_root/Cargo.toml")
  eval "$old_nullglob"
}

# Whether $1 is excluded from taking `goodfetch-dev-workspace-hack`: the
# protocol crate (linked by the Kennel's workspace, ADR 0141) and the two
# workspace-hack crates themselves.
is_excluded_member() {
  case "$1" in
    goodfetch-protocol | goodfetch-workspace-hack | goodfetch-dev-workspace-hack) return 0 ;;
    *) return 1 ;;
  esac
}

# Inspects a member manifest at $1 for `goodfetch-dev-workspace-hack`: prints
# `in_dev=0|1 outside_dev=0|1`. Handles flat keys, dotted dependency subtables
# (`[dev-dependencies.goodfetch-dev-workspace-hack]`) and renamed dependencies
# (`package = "goodfetch-dev-workspace-hack"`).
inspect_dev_hack_dep() {
  awk -v pkg="$dev_hack_pkg" '
    BEGIN { section = ""; in_dev = 0; outside_dev = 0 }
    /^[[:space:]]*\[/ {
      line = $0
      sub(/#.*/, "", line)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
      section = line
      if (section == "[dev-dependencies." pkg "]") {
        in_dev = 1
      } else if (section ~ ("^\\[(target\\..*\\.)?(dependencies|build-dependencies|dev-dependencies)\\." pkg "\\]$")) {
        outside_dev = 1
      }
      next
    }
    {
      line = $0
      sub(/#.*/, "", line)
      if (line ~ ("^[[:space:]]*" pkg "([[:space:]]*=|[[:space:]]*\\.)") || line ~ ("package[[:space:]]*=[[:space:]]*[\"\x27]" pkg "[\"\x27]")) {
        if (section == "[dev-dependencies]" || section ~ /^\[dev-dependencies\.[^]]+\]$/) {
          in_dev = 1
        } else if (section ~ /^\[(target\..*\.)?(dependencies|build-dependencies|dev-dependencies)(\.[^]]+)?\]$/) {
          outside_dev = 1
        }
      }
    }
    END { printf "in_dev=%d outside_dev=%d\n", in_dev, outside_dev }
  ' "$1"
}

# Adds `goodfetch-dev-workspace-hack.workspace = true` to `[dev-dependencies]`
# of the manifest at $1 when it is missing.
add_dev_hack_dep() {
  local target=$1
  awk -v dep="$dev_hack_pkg.workspace = true" '
    BEGIN { in_dev = 0; inserted = 0; saw_dev = 0; trailing = "" }
    /^\[dev-dependencies\][[:space:]]*(#.*)?$/ {
      in_dev = 1
      saw_dev = 1
      print
      next
    }
    /^\[/ {
      if (in_dev && !inserted) {
        print dep
        if (trailing != "") {
          printf "%s", trailing
          trailing = ""
        } else {
          print ""
        }
        inserted = 1
        in_dev = 0
      }
      if (!saw_dev && !inserted && /^\[lints\]/) {
        print "[dev-dependencies]"
        print dep
        print ""
        inserted = 1
      }
      print
      next
    }
    {
      if (in_dev && ($0 ~ /^[[:space:]]*$/ || $0 ~ /^[[:space:]]*#/)) {
        trailing = trailing $0 "\n"
        next
      }
      if (trailing != "") {
        printf "%s", trailing
        trailing = ""
      }
      print
    }
    END {
      if (in_dev && !inserted) {
        print dep
        if (trailing != "") {
          printf "%s", trailing
        }
        inserted = 1
      } else if (!inserted) {
        if (trailing != "") {
          printf "%s", trailing
        } else {
          print ""
        }
        print "[dev-dependencies]"
        print dep
      }
    }
  ' "$target" >"$target.tmp"
  mv "$target.tmp" "$target"
}

# Ensures every non-excluded member takes `goodfetch-dev-workspace-hack` in
# `[dev-dependencies]`.
manage_dev_deps() {
  local ws_root=$1 name member_manifest state in_dev outside_dev
  while IFS=$'\t' read -r name member_manifest; do
    if is_excluded_member "$name"; then
      continue
    fi
    state=$(inspect_dev_hack_dep "$member_manifest")
    in_dev=${state#*in_dev=}
    in_dev=${in_dev%% *}
    outside_dev=${state#*outside_dev=}
    if [[ "$outside_dev" -ne 0 ]]; then
      echo "error[workspace-hack] $name declares $dev_hack_pkg outside [dev-dependencies] ($member_manifest)" >&2
      return 1
    fi
    if [[ "$in_dev" -eq 0 ]]; then
      add_dev_hack_dep "$member_manifest"
    fi
  done < <(workspace_members "$ws_root")
}

# Verifies read-only that every non-excluded member takes
# `goodfetch-dev-workspace-hack` in `[dev-dependencies]` only, and excluded
# members do not take it at all.
check_dev_deps() {
  local ws_root=$1 name member_manifest state in_dev outside_dev status=0
  while IFS=$'\t' read -r name member_manifest; do
    state=$(inspect_dev_hack_dep "$member_manifest")
    in_dev=${state#*in_dev=}
    in_dev=${in_dev%% *}
    outside_dev=${state#*outside_dev=}
    if is_excluded_member "$name"; then
      if [[ "$in_dev" -ne 0 || "$outside_dev" -ne 0 ]]; then
        echo "error[workspace-hack] excluded member $name must not depend on $dev_hack_pkg ($member_manifest)" >&2
        status=1
      fi
    elif [[ "$outside_dev" -ne 0 ]]; then
      echo "error[workspace-hack] $name declares $dev_hack_pkg outside [dev-dependencies]: move it to [dev-dependencies]" >&2
      status=1
    elif [[ "$in_dev" -eq 0 ]]; then
      echo "error[workspace-hack] $name does not depend on $dev_hack_pkg in [dev-dependencies]: run ci/workspace-hack.sh generate" >&2
      status=1
    fi
  done < <(workspace_members "$ws_root")
  return "$status"
}

generate() {
  clear_hakari_section "$dev_manifest"
  (cd "$root" && cargo hakari generate --quiet && cargo hakari manage-deps --quiet --yes)
  write_code_side "$manifest" "$dev_manifest"
  keep_build_side "$manifest"
  manage_dev_deps "$root"
  (cd "$root" && cargo metadata --offline --format-version 1 >/dev/null)
}

# Makes $2 a folder holding a link to each entry of the folder $1 but the
# target folder and Git's.
link_entries() {
  local entry
  mkdir "$2"
  for entry in "$1"/* "$1"/.[!.]*; do
    [[ -e $entry || -L $entry ]] || continue
    case ${entry##*/} in
      target | .git) ;;
      *) ln -s "$entry" "$2/${entry##*/}" ;;
    esac
  done
}

# Replaces symlinks along $2 under mirror $1 with real folders of links so
# manifests beneath $2 can be copied without touching $root.
materialize_dir() {
  local folder=$1 rel=$2 part path=""
  for part in ${rel//\// }; do
    path=${path:+$path/}$part
    if [[ -L "$folder/$path" ]]; then
      rm "$folder/$path"
      link_entries "$root/$path" "$folder/$path"
    fi
  done
}

# Fills the folder $1 with the workspace's mirror.
mirror() {
  local folder=$1 path
  rmdir "$folder"
  link_entries "$root" "$folder"
  materialize_dir "$folder" "$hack"
  materialize_dir "$folder" "$dev_hack"
  for path in Cargo.lock "$hack/Cargo.toml" "$dev_hack/Cargo.toml"; do
    rm "$folder/$path"
    cp -p "$root/$path" "$folder/$path"
  done
}

case "${1:-}" in
  generate)
    generate
    ;;
  check)
    copy=$(mktemp -d)
    trap 'rm -rf "$copy"' EXIT
    mirror "$copy"
    clear_hakari_section "$copy/$dev_hack/Cargo.toml"
    (cd "$copy" && cargo hakari generate --quiet)
    write_code_side "$copy/$hack/Cargo.toml" "$copy/$dev_hack/Cargo.toml"
    keep_build_side "$copy/$hack/Cargo.toml"
    failed=0
    if ! diff -u --label "$hack/Cargo.toml" --label "$hack/Cargo.toml (generated)" "$manifest" "$copy/$hack/Cargo.toml"; then
      echo "error[workspace-hack] $hack/Cargo.toml is out of date: run ci/workspace-hack.sh generate" >&2
      failed=1
    fi
    if ! diff -u --label "$dev_hack/Cargo.toml" --label "$dev_hack/Cargo.toml (generated)" "$dev_manifest" "$copy/$dev_hack/Cargo.toml"; then
      echo "error[workspace-hack] $dev_hack/Cargo.toml is out of date: run ci/workspace-hack.sh generate" >&2
      failed=1
    fi
    if ! (cd "$copy" && cargo hakari manage-deps --dry-run --quiet); then
      echo "error[workspace-hack] a member does not depend on goodfetch-workspace-hack: run ci/workspace-hack.sh generate" >&2
      failed=1
    fi
    if ! check_dev_deps "$root"; then
      failed=1
    fi
    exit "$failed"
    ;;
  *)
    echo "usage: ci/workspace-hack.sh generate | check" >&2
    exit 2
    ;;
esac
