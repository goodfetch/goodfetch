#!/usr/bin/env bash
# One skew job of the Release suite in CI (ADR 0066), by its name in
# ci/release/skew-jobs.json, which `xtask compat jobs` writes:
#   build <job>  before the deny: builds both Products' executables and keeps
#                a copy of each, which the runs below never rebuild; for a
#                released minor, also fetches its tag's dependencies, through
#                the tag's own `just fetch`, and, for each older-engine step,
#                keeps the minor's executable of its Product, from the
#                artifact set `just artifacts` keeps;
#   run <job>    runs each of the job's steps, the Journeys `just journeys`
#                chooses against the current executable of their Product, or
#                `just older-engine` on the minor's executable of its Product
#                (the current build's own while no minor is released), and
#                fails on a step none of whose tests ran. A failed step
#                stops none after it. It first prints the checks that await
#                their harness, which nothing counts as passed.
# `run` goes under ADR 0043's deny through ci/egress/under-deny.sh, which the
# workflow step names. Each step's JUnit files land in target/junit/ as
# `<runner>-<checks>.xml`.
set -euo pipefail

part=${1:?usage: ci/release/skew.sh build|run <job>}
name=${2:?usage: ci/release/skew.sh build|run <job>}
jobs=ci/release/skew-jobs.json
kept=$PWD/target/release-suite/skew

job=$(jq -c --arg name "$name" '.jobs[] | select(.name == $name)' "$jobs")
if [[ -z "$job" ]]; then
  echo "error: $jobs has no job $name" >&2
  exit 2
fi
tag=$(jq -r '.tag // ""' <<<"$job")

case "$part" in
  build)
    mkdir -p "$kept"
    cargo run --quiet --locked --bin xtask -- test-tools build --cargo-profile ci-test \
      --tool collie --tool foxhound --out "$kept/test-tools.json"
    for product in collie foxhound; do
      cp "$(jq -er --arg product "$product" '.[$product]' "$kept/test-tools.json")" \
        "$kept/goodfetch-$product"
    done
    if [[ -n "$tag" ]]; then
      scratch=$(mktemp -d)
      cleanup() {
        if [[ -d "$scratch/tree" ]]; then git worktree remove --force "$scratch/tree"; fi
        rm -rf "$scratch"
      }
      trap cleanup EXIT
      git worktree add --quiet --detach "$scratch/tree" "$tag^{commit}"
      (cd "$scratch/tree" && just fetch)
      # The skew jobs run on linux-x64-gnu. A release's executable is an
      # archive holding `goodfetch-<product>`, or that file itself; a set
      # without it fails the job, never skips the check.
      version=$(jq -r .version <<<"$job")
      products=$(jq -r '.steps[] | .older_engine // empty' <<<"$job")
      if [[ -n "$products" ]]; then
        listed=$(just artifacts)
        set_folder=$(dirname "$(tail -n 1 <<<"$listed")")
        mkdir -p "$kept/older"
        for product in $products; do
          folder="$set_folder/$version/$product/executables/x86_64-unknown-linux-gnu"
          artifact=""
          for file in "$folder"/*; do
            case "$file" in
              *.sigstore.json | *.sigstore.jsonl) ;;
              *) [[ ! -f "$file" ]] || artifact=$file ;;
            esac
          done
          if [[ -z "$artifact" ]]; then
            echo "error: the artifact set holds no $version executable of $product in $folder" >&2
            exit 1
          fi
          unpacked="$scratch/older-$product"
          mkdir -p "$unpacked"
          case "$artifact" in
            *.tar.* | *.tgz) tar -xf "$artifact" -C "$unpacked" ;;
            *.zip) unzip -q "$artifact" -d "$unpacked" ;;
            *) cp "$artifact" "$unpacked/goodfetch-$product" ;;
          esac
          executable=$(find "$unpacked" -type f -name "goodfetch-$product" | head -n 1)
          if [[ -z "$executable" ]]; then
            echo "error: $artifact holds no goodfetch-$product" >&2
            exit 1
          fi
          install -m 0755 "$executable" "$kept/older/goodfetch-$product"
        done
      fi
    fi
    ;;
  run)
    jq -r '.awaiting[] | "Not run, awaiting #\(.issue): \(.check)"' "$jobs"
    mkdir -p target/junit
    status=0
    steps=$(jq '.steps | length' <<<"$job")
    for ((index = 0; index < steps; index++)); do
      step=$(jq -c --argjson index "$index" '.steps[$index]' <<<"$job")
      checks=$(jq -r '.checks | join("+")' <<<"$step")
      older=$(jq -r '.older_engine // ""' <<<"$step")
      environment=()
      if [[ -n "$older" ]]; then
        command=(just older-engine "$older")
        [[ -z "$tag" ]] || command+=(--engine "$kept/older/goodfetch-$older")
      else
        command=(just journeys)
        [[ -z "$tag" ]] || command+=(--tag "$tag")
        while IFS= read -r argument; do command+=("$argument"); done < <(jq -r '.journeys[]' <<<"$step")
        command+=(--engine "$kept/$(jq -r .executable <<<"$step")")
        if [[ $(jq -r .stdio <<<"$step") == true ]]; then
          environment=(
            GOODFETCH_COLLIE_ENGINE_PATH="$kept/goodfetch-collie"
            GOODFETCH_FOXHOUND_ENGINE_PATH="$kept/goodfetch-foxhound"
          )
        fi
      fi
      echo "Skew $name, $checks: ${command[*]}"
      reports="$kept/junit/$checks"
      rm -rf "$reports"
      mkdir -p "$reports"
      env ${environment[@]+"${environment[@]}"} "${command[@]}" --junit "$reports" || {
        echo "Skew $name, $checks: failed" >&2
        status=1
      }
      ran=0
      for report in "$reports"/*.xml; do
        [[ -f "$report" ]] || continue
        cases=$({ grep -o '<testcase' "$report" || true; } | wc -l)
        skipped=$({ grep -o '<skipped' "$report" || true; } | wc -l)
        ran=$((ran + cases - skipped))
        cp "$report" "target/junit/$(basename "$report" .xml)-$checks.xml"
      done
      # A selection that holds no test would otherwise pass (`just test` sets
      # NEXTEST_NO_TESTS=pass), and the check with it.
      if [[ $ran -eq 0 ]]; then
        echo "error: skew $name, $checks: no test ran" >&2
        status=1
      fi
    done
    exit "$status"
    ;;
  *)
    echo "error: the part is build or run, not $part" >&2
    exit 2
    ;;
esac
