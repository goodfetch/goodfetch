#!/usr/bin/env bash
# `just fuzz-nightly [ref]`: the nightly fuzzing the gate host's cron starts
# (TESTING.md, Fuzzing; the entry in ci/fuzz-nightly.crontab), on `main`'s
# head by default, in a worktree of its own (ci/gate-worktree.sh's
# `fuzz-nightly`, whose lock it takes).
#
# It runs every target of every fuzz crate (a tracked `fuzz/Cargo.toml` whose
# package carries `cargo-fuzz = true`, as ci/fuzz-check.sh finds them) with
# `cargo fuzz run`, on the toolchain and cargo-fuzz of
# ci/sanitizer-toolchain.toml, at the lowest CPU and disk priority, on
# GOODFETCH_FUZZ_CORES cores (4) for GOODFETCH_FUZZ_SECONDS seconds (3600) in
# all: as many targets run at once as there are cores, and each runs for the
# same share, the hour divided by the rounds the cores take. Each target's
# corpus is kept between nights on the volume, in `<log>/corpus/<crate>/
# <target>/`, listed first so a run grows it and never the committed seeds,
# and is pruned with `cargo fuzz cmin` after the run. The build and the runs
# get no token and no `.env`: only the variables `untrusted` keeps.
#
# It gives way to a gate: a night that finds `just merge`'s gate running, or
# the backstop or a test-health run in the backstop's worktree, starts nothing
# and says it skipped. A gate that starts during a night stops it
# (ci/merge-pr.sh): on TERM, HUP or INT the job stops every process it
# started, waits for them, writes its line and only then lets its lock go,
# which ends the gate's wait.
#
# A crash, a leak, a timeout or an out-of-memory is shrunk with `cargo fuzz
# tmin` and kept in `<log>/crashes/<crate>/<target>/`. With a clone of the
# private fuzzing repository at GOODFETCH_FUZZ_REPOSITORY (~/goodfetch-fuzzing),
# it is filed there as an issue with its input, the night's pruned corpora
# and the kept inputs are pushed there, and a ticket of the main tracker
# names the target and links that issue, without the input; while that
# target's ticket is open, a comment on it does instead. Without a clone, the
# ticket says where the host keeps the input, and the corpora stay on the
# volume. A stop that comes while the night files waits for the filing.
# Nothing here changes a gate's verdict.
#
# Each night appends one line to `<log>/results.txt`, `<UTC day> <commit>
# fuzz=<pass|crash (…)|fail (…)|stopped|skipped (…)>`, followed, for a night
# that ran, by `targets=<n> seconds=<each target's share> grew=<targets whose
# corpus grew>/<n> corpora=<pushed|local (…)>`; a night that ends on its
# setup reads `fail`. Its log is `<log>/<day>.log`, each target's output in
# `<log>/<day>/`, with `<log>` GOODFETCH_FUZZ_LOG or ~/fuzz-nightly. It fails
# unless the night passed. A night is the scripts of the commit it fuzzes,
# whichever script started it: nothing fast-forwards the checkout cron starts
# in.
set -euo pipefail

ref=${1:-origin/main}
logs=${GOODFETCH_FUZZ_LOG:-$HOME/fuzz-nightly}
mkdir -p "$logs"
day=$(date -u +%F)
exec >>"$logs/$day.log" 2>&1
echo "== $(date -u +%FT%TZ) the nightly fuzzing on $ref"

commit=unknown
said=""
locked=""
scripts=${GOODFETCH_GATE_SCRIPTS:-}
result() {
  echo "$day $commit fuzz=$1${2:+ $2}" >>"$logs/results.txt"
  said=yes
}
# `set -e` ends the night on a setup that fails: its line still says so. The
# lock goes last, after the line.
trap '[[ -n $said ]] || result fail; [[ -z $locked ]] || gate_worktree_unlock; [[ -z $scripts ]] || rm -rf "$scripts"' EXIT

git fetch --quiet origin
commit=$(git rev-parse --verify "$ref^{commit}")
if [[ ${GOODFETCH_GATE_SCRIPTS_OF:-} != "$commit" ]]; then
  scripts=$(mktemp -d)
  git archive "$commit" ci | tar -x -C "$scripts"
  GOODFETCH_GATE_SCRIPTS_OF=$commit GOODFETCH_GATE_SCRIPTS=$scripts \
    exec bash "$scripts/ci/fuzz-nightly.sh" "$commit" </dev/null
fi
unset GOODFETCH_GATE_SCRIPTS GOODFETCH_GATE_SCRIPTS_OF

gate_worktree_name="fuzz-nightly"
# shellcheck source=ci/gate-worktree.sh
source "$(dirname "$0")/gate-worktree.sh"

# Every long step runs in a process group of its own (`set -m`), in the
# background, while the job waits for it: a stop is handled at once, not
# when the step ends, and ends the whole group.
set -m
groups=()
stop() {
  trap '' TERM HUP INT
  echo "== $(date -u +%FT%TZ) stopped: ending what the night started"
  local group tries
  for group in ${groups[@]+"${groups[@]}"}; do
    kill -TERM -- "-$group" 2>/dev/null || true
  done
  for group in ${groups[@]+"${groups[@]}"}; do
    wait "$group" 2>/dev/null || true
    tries=0
    while kill -0 -- "-$group" 2>/dev/null; do
      tries=$((tries + 1))
      if ((tries == 300)); then kill -KILL -- "-$group" 2>/dev/null || true; fi
      sleep 0.1
    done
  done
  result stopped
  exit 143
}
trap stop TERM HUP INT

# Runs `$@` in folder `$1`, its output appended to file `$2`, as a process
# group of its own, and waits for it; its status is the step's.
step() {
  local folder=$1 output=$2 pid status=0
  shift 2
  (cd "$folder" && "$@") >>"$output" 2>&1 &
  pid=$!
  groups=("$pid")
  wait "$pid" || status=$?
  groups=()
  return "$status"
}

# Runs `$@` at the lowest priority, with no variable but those that find
# the toolchain and name the user: no token, nothing of `.env`. Its HOME is
# an empty folder of the night's, so nothing it runs finds gh's or git's
# configuration, which holds the token, under the user's; the toolchain is
# found through CARGO_HOME and RUSTUP_HOME, set to the user's.
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
untrusted() {
  local keep=() name lower=(nice -n 19)
  for name in PATH USER LOGNAME LANG LC_ALL TMPDIR CARGO_BUILD_JOBS; do
    [[ -z ${!name:-} ]] || keep+=("$name=${!name}")
  done
  if command -v ionice >/dev/null; then lower+=(ionice -c 3); fi
  env -i ${keep[@]+"${keep[@]}"} HOME="$night/home" CARGO_HOME="$cargo_home" RUSTUP_HOME="$rustup_home" \
    "${lower[@]}" "$@"
}

holder=$(gate_worktree_holder "$lock")
if [[ -n $holder ]]; then
  echo "== skipped: process $holder holds $worktree"
  result "skipped (another nightly fuzzing runs)"
  exit 1
fi
gate_worktree_lock
locked=yes
# The job takes its lock before it reads the gate's, and a gate takes its own
# before it reads this one (ci/merge-pr.sh): of a night and a gate starting
# together, at least one sees the other. A night that sees a gate skips; a
# gate that sees a night stops it.
holder=$(merge_gate_holder)
if [[ -n $holder ]]; then
  echo "== skipped: the merge gate, process $holder, is running"
  result "skipped (the merge gate is running)"
  exit 1
fi
holder=$(gate_worktree_holder "$root/.claude/worktrees/gate-backstop.lock")
if [[ -n $holder ]]; then
  echo "== skipped: process $holder holds the backstop's worktree"
  result "skipped (the backstop's worktree is busy)"
  exit 1
fi

if [[ -e "$worktree/.git" ]]; then
  git -C "$worktree" checkout --quiet --force --detach "$commit"
  git -C "$worktree" clean -ffdxq -e /target/
else
  git worktree add --quiet --lock --detach "$worktree" "$commit"
fi
cd "$worktree"
toolchain=$(sed -n 's/^toolchain = "\(.*\)"$/\1/p' ci/sanitizer-toolchain.toml)
cores=${GOODFETCH_FUZZ_CORES:-4}
seconds=${GOODFETCH_FUZZ_SECONDS:-3600}
night="$logs/$day"
rm -rf "$night"
mkdir -p "$night/home"

# The targets, from the fuzz crates' manifests: `crates[i]` is the fuzz
# crate's folder, `names[i]` the target, `keys[i]` `<crate>/<target>`, the
# crate named by the folder that holds `fuzz/`.
crates=() names=() keys=()
while IFS= read -r manifest; do
  grep -q '^cargo-fuzz = true' "$manifest" || continue
  while IFS= read -r name; do
    crates+=("$(dirname "$manifest")")
    names+=("$name")
    keys+=("$(basename "$(dirname "$(dirname "$manifest")")")/$name")
  done < <(awk '/^\[\[bin\]\]/ { bin = 1; next } /^\[/ { bin = 0 }
    bin && /^name *= *"/ { sub(/^name *= *"/, ""); sub(/".*/, ""); print }' "$manifest")
done < <(git ls-files -- 'fuzz/Cargo.toml' '*/fuzz/Cargo.toml')
count=${#names[@]}
if ((count == 0)); then
  echo "== no fuzz target"
  result fail "targets=0"
  exit 1
fi
rounds=$(((count + cores - 1) / cores))
share=$((seconds / rounds))
lanes=$((count < cores ? count : cores))
echo "== $count targets, $lanes at once, $share seconds each, on $toolchain"

# Each crate builds first, outside the targets' time, into a folder of its
# own; a crate that does not build runs no target, and fails the night.
failed=()
built=" "
for crate in $(printf '%s\n' "${crates[@]}" | sort -u); do
  echo "== $(date -u +%FT%TZ) building $crate"
  if CARGO_BUILD_JOBS=$cores step "$(dirname "$crate")" "$night/build-${crate//\//-}.log" \
    untrusted cargo "+$toolchain" fuzz build --fuzz-dir fuzz --target-dir "$worktree/target/fuzz/$crate"; then
    built="$built$crate "
  else
    failed+=("build of $crate")
  fi
done

files() { find "$1" -type f 2>/dev/null | wc -l | tr -d ' '; }

# Runs target `$1` for its share; notes its corpus's size before the run and
# its status in `<night>/status/<key>`.
run_target() {
  local i=$1 crate=${crates[$1]} name=${names[$1]} key=${keys[$1]}
  local corpus="$logs/corpus/$key" artifacts="$night/artifacts/$key/" before status=0
  local inputs=("$corpus") options=("-max_total_time=$share" "-artifact_prefix=$artifacts")
  mkdir -p "$corpus" "$artifacts" "$night/status/${key%/*}" "$night/logs/${key%/*}"
  before=$(files "$corpus")
  if [[ -d $crate/$name/seeds ]]; then inputs+=("fuzz/$name/seeds"); fi
  if [[ -f $crate/$name/dictionary.dict ]]; then options+=("-dict=fuzz/$name/dictionary.dict"); fi
  (cd "$(dirname "$crate")" && untrusted cargo "+$toolchain" fuzz run --fuzz-dir fuzz \
    --target-dir "$worktree/target/fuzz/$crate" "$name" "${inputs[@]}" -- "${options[@]}") \
    >"$night/logs/$key.log" 2>&1 || status=$?
  echo "$before $status" >"$night/status/$key"
}

# Lane `$1` runs every `lanes`th target from its own, one after the other.
run_lane() {
  local i
  for ((i = $1; i < count; i += lanes)); do
    if [[ $built == *" ${crates[$i]} "* ]]; then run_target "$i"; fi
  done
}

echo "== $(date -u +%FT%TZ) fuzzing"
lane_groups=()
for ((lane = 0; lane < lanes; lane++)); do
  run_lane "$lane" &
  lane_groups+=("$!")
  groups=("${lane_groups[@]}")
done
groups=("${lane_groups[@]}")
for group in "${lane_groups[@]}"; do wait "$group" || true; done
groups=()

# What each target left: a crash (`crash-`, `leak-`, `timeout-` or `oom-`
# artifact), a run that failed without one, a corpus that grew.
crashed=() grew=0
for ((i = 0; i < count; i++)); do
  key=${keys[$i]}
  [[ -f $night/status/$key ]] || continue
  read -r before status <"$night/status/$key"
  artifact=$(find "$night/artifacts/$key" -type f \( -name 'crash-*' -o -name 'leak-*' \
    -o -name 'timeout-*' -o -name 'oom-*' \) 2>/dev/null | sort | head -n 1)
  if [[ -n $artifact ]]; then
    crashed+=("$i:$artifact")
  elif ((status != 0)); then
    failed+=("${names[$i]}")
  fi
  if (($(files "$logs/corpus/$key") > before)); then grew=$((grew + 1)); fi
done

echo "== $(date -u +%FT%TZ) pruning the corpora"
for ((i = 0; i < count; i++)); do
  key=${keys[$i]} crate=${crates[$i]}
  [[ $built == *" $crate "* && $(files "$logs/corpus/$key") -gt 0 ]] || continue
  step "$(dirname "$crate")" "$night/logs/$key.log" untrusted cargo "+$toolchain" fuzz cmin \
    --fuzz-dir fuzz --target-dir "$worktree/target/fuzz/$crate" "${names[$i]}" "$logs/corpus/$key" ||
    echo "warning: cmin of $key failed; its corpus stays unpruned"
done

# Shrinks each crash and keeps it: each entry of `kept` is `<i>:<input>`,
# target `i`'s kept input.
kept=()
for entry in ${crashed[@]+"${crashed[@]}"}; do
  i=${entry%%:*} artifact=${entry#*:}
  key=${keys[$i]} crate=${crates[$i]}
  echo "== $(date -u +%FT%TZ) shrinking the crash of $key"
  shrunk="$night/artifacts/$key.min"
  step "$(dirname "$crate")" "$night/logs/$key.log" untrusted cargo "+$toolchain" fuzz tmin \
    --fuzz-dir fuzz --target-dir "$worktree/target/fuzz/$crate" "${names[$i]}" "$artifact" -- \
    "-exact_artifact_path=$shrunk" || true
  [[ -s $shrunk ]] || shrunk=$artifact
  mkdir -p "$logs/crashes/$key"
  input="$logs/crashes/$key/$day-$(basename "$artifact")"
  cp "$shrunk" "$input"
  kept+=("$i:$input")
done

# From here on the job holds the user's environment: it files and pushes. A
# stop asked now waits for the filing, so no private issue is left without
# its ticket and the line says what was filed; each network call is bounded,
# so that wait is too.
stop_asked=""
trap 'stop_asked=yes' TERM HUP INT
bounded=()
if command -v timeout >/dev/null; then bounded=(timeout 300); fi
clone=${GOODFETCH_FUZZ_REPOSITORY:-$HOME/goodfetch-fuzzing}
have_clone=""
if [[ -e $clone/.git ]]; then have_clone=yes; fi

# The clone's `corpus/` and `crashes/` become the volume's, in one commit.
# Called as a condition, where `set -e` stops nothing: each step says so.
push_corpora() {
  local folder
  ${bounded[@]+"${bounded[@]}"} git -C "$clone" pull --quiet --ff-only || return 1
  for folder in corpus crashes; do
    rm -rf "${clone:?}/$folder" || return 1
    if [[ -d $logs/$folder ]]; then cp -R "$logs/$folder" "$clone/$folder" || return 1; fi
  done
  git -C "$clone" add --all . || return 1
  git -C "$clone" diff --cached --quiet ||
    git -C "$clone" commit --quiet --message "The corpora of $day, at $commit" || return 1
  ${bounded[@]+"${bounded[@]}"} git -C "$clone" push --quiet origin HEAD
}
corpora="local (no clone of the fuzzing repository)"
if [[ -n $have_clone ]]; then
  echo "== $(date -u +%FT%TZ) pushing the corpora"
  if push_corpora; then corpora=pushed; else corpora="local (the push failed)"; fi
fi

# Runs gh in folder `$1`, bounded.
gh_in() {
  local folder=$1
  shift
  (cd "$folder" && ${bounded[@]+"${bounded[@]}"} gh "$@")
}

crashes=()
for entry in ${kept[@]+"${kept[@]}"}; do
  i=${entry%%:*} input=${entry#*:}
  name=${names[$i]} key=${keys[$i]} crate=${crates[$i]}
  title="Fuzz crash in $name"
  links=() private=""
  if [[ -n $have_clone ]]; then
    body=$(mktemp)
    {
      echo "The nightly fuzzing of $day found a crash in the fuzz target \`$name\` (\`$crate\`), at $commit."
      echo
      if (($(wc -c <"$input") <= 40000)); then
        echo "Its shrunk input, in base64:"
        echo
        echo '```'
        base64 <"$input"
        echo '```'
        echo
      fi
      echo "The input is in this repository at \`crashes/$key/$(basename "$input")\`. Replay it from \`$(dirname "$crate")\`:"
      echo
      echo "    cargo +$toolchain fuzz run --fuzz-dir fuzz $name <input>"
      echo
      echo "The end of the run's output:"
      echo
      echo '```'
      tail -n 60 "$night/logs/$key.log"
      echo '```'
    } >"$body"
    if private=$(gh_in "$clone" issue create --title "Crash in $name ($day)" --body-file "$body"); then
      links+=("$private")
    else
      private=""
    fi
    rm -f "$body"
  fi
  body=$(mktemp)
  {
    echo "The nightly fuzzing of $day found a crash in the fuzz target \`$name\` (\`$crate\`), at $commit."
    echo
    if [[ -n $private ]]; then
      echo "Its shrunk input and the run's output are filed in the private fuzzing repository: $private. This ticket holds no input (TESTING.md, Fuzzing)."
    else
      echo "Its shrunk input is kept on the gate host, at \`$input\`, until the private fuzzing repository holds it (#2456). This ticket holds no input (TESTING.md, Fuzzing)."
    fi
  } >"$body"
  # The target's open ticket, found by its exact title, takes a comment in
  # place of a second ticket. A search that fails finds none.
  open=$(gh_in "$root" issue list --state open --search "\"$title\" in:title" --json number,title \
    --jq "map(select(.title == \"$title\")) | .[0].number // empty") || {
    echo "warning: the search for an open ticket of $name failed; filing a new one"
    open=""
  }
  if [[ -n $open ]]; then
    if ticket=$(gh_in "$root" issue comment "$open" --body-file "$body"); then links+=("$ticket"); fi
  elif ticket=$(gh_in "$root" issue create --title "$title" --label ready-for-agent --body-file "$body"); then
    links+=("$ticket")
  fi
  rm -f "$body"
  if ((${#links[@]} == 0)); then
    crashes+=("$name: not filed")
  else
    crashes+=("$name: $(printf '%s, ' "${links[@]}" | sed 's/, $//')")
  fi
done

[[ -z $stop_asked ]] || echo "== a stop came while the night filed; the night ends now"
summary="targets=$count seconds=$share grew=$grew/$count corpora=$corpora"
if ((${#crashes[@]} > 0)); then
  result "crash ($(printf '%s; ' "${crashes[@]}" | sed 's/; $//'))" "$summary"
  exit 1
elif ((${#failed[@]} > 0)); then
  result "fail ($(printf '%s, ' "${failed[@]}" | sed 's/, $//'))" "$summary"
  exit 1
fi
result pass "$summary"
