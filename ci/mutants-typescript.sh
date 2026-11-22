#!/usr/bin/env bash
# ADR 0072's mutation testing of the TypeScript SDK's hand-written runtime
# (TESTING.md, Feature coverage): StrykerJS with its Vitest runner, through
# sdks/typescript/stryker.config.mjs.
#   ci/mutants-typescript.sh <base>   the in-diff run (`just mutants-typescript`):
#                                     only the runtime lines HEAD changed since
#                                     <base>, as StrykerJS's mutation ranges;
#                                     its report goes to target/mutants-typescript/;
#   ci/mutants-typescript.sh          the full run (`just mutants-sdks typescript`):
#                                     the whole runtime; its report goes to
#                                     target/mutants-sdks/typescript/.
# Neither mutates what `xtask codegen` writes (ADR 0099) nor a test. Each
# lists the mutants that survived, and those no test reaches, in its
# report's summary.md, for the pull request's Evidence section, and fails on
# none (ADR 0072): it fails only when StrykerJS cannot judge a mutant, as
# when the unmutated tests fail. It builds the test tools through `$JUST`,
# once its checks pass.
set -euo pipefail

base=${1:-}
cd "$(git rev-parse --show-toplevel)"
mutate=()
if [[ -n "$base" ]]; then
  reports=target/mutants-typescript
  mkdir -p "$reports"
  # The runtime as the config's `mutate` names it, which a `--mutate` list
  # replaces, its exclusions with it.
  runtime=(
    ':(glob)sdks/typescript/*/src/**/*.ts'
    ':(exclude,glob)**/generated/**'
    ':(exclude,glob)**/*.test.ts'
    ':(exclude,glob)**/*.test-d.ts'
  )
  # The ranges are HEAD's lines, which uncommitted changes would move.
  if ! git diff --quiet HEAD -- "${runtime[@]}"; then
    echo "error: commit the TypeScript changes first: the mutation ranges are HEAD's lines" >&2
    exit 2
  fi
  # Paths as written, unquoted, so a name with a space or a non-ASCII
  # letter reads whole.
  git -c core.quotePath=false diff --no-ext-diff --no-color --unified=0 \
    --src-prefix=a/ --dst-prefix=b/ "$base" HEAD -- "${runtime[@]}" >"$reports/change.diff"
  # Each hunk's lines on HEAD's side, as `<file>:<first>-<last>`; a hunk
  # that only removes lines, or a deleted file, has none. StrykerJS tests a
  # mutant only when it lies wholly within one range.
  ranges=$(awk '
    /^\+\+\+ / {
      file = substr($0, 5); sub(/\t$/, "", file)
      file = (file == "/dev/null") ? "" : substr(file, 3)
      next
    }
    /^@@ / && file != "" {
      count = split(substr($3, 2), side, ",")
      lines = (count == 2) ? side[2] : 1
      if (lines > 0) print file ":" side[1] "-" (side[1] + lines - 1)
    }
  ' "$reports/change.diff" | paste -sd, -)
  if [[ -z "$ranges" ]]; then
    echo "The change touches no line of the TypeScript SDK's runtime: no mutant to test."
    exit 0
  fi
  mutate=(--mutate "$ranges")
else
  reports=target/mutants-sdks/typescript
  mkdir -p "$reports"
fi
# The config writes its reports there.
export GOODFETCH_STRYKER_REPORTS=$reports

# The tests find the commit's executables as under `just test`.
export GOODFETCH_TEST_SUITE=pre-merge
"${JUST:-just}" _test-tools
export GOODFETCH_TEST_TOOLS="$PWD/target/test-tools.json"

# StrykerJS sets Vitest's name pattern itself, so its setup file
# (sdks/typescript/vitest.stryker.setup.ts) skips the tests the quarantine's
# pattern does not match (ADR 0064).
exclusions=$(cargo run --quiet --bin xtask -- quarantine exclude)
pattern=$(jq -r '.vitest[]? | ltrimstr("--testNamePattern=")' <<<"$exclusions")
if [[ -n "$pattern" ]]; then export GOODFETCH_STRYKER_NAME_PATTERN="$pattern"; fi
# Each StrykerJS test runner writes `stryker-setup-<n>.js` into the working
# directory and deletes it when it stops, unless StrykerJS kills it first
# (StrykerJS 10.1.1).
status=0
pnpm exec stryker run sdks/typescript/stryker.config.mjs ${mutate[@]+"${mutate[@]}"} || status=$?
rm -f stryker-setup-*.js
[[ $status -eq 0 ]] || exit "$status"

{
  echo "### Surviving TypeScript mutants"
  echo
  jq -r '
    [.files | to_entries[] | .key as $file | .value.mutants[]
      | select(.status == "Survived" or .status == "NoCoverage")
      | "- `\($file):\(.location.start.line):\(.location.start.column)` \(.mutatorName): `\(.replacement)`, "
        + (if .status == "Survived" then "survived" else "no test reaches it" end)]
    | if length == 0 then "None." else .[] end
  ' "$reports/report.json"
} | tee "$reports/summary.md"
