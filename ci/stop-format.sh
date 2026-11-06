#!/usr/bin/env bash
# The agents' Stop hook (.claude/settings.json): before an agent ends its
# turn, the formatting of the files its working tree changed against HEAD,
# by the formatters `just fmt-check` runs, in seconds and offline. Rust
# runs `cargo fmt --check` over the workspace a changed `.rs` file is in
# (about 5 s), since `cargo fmt` alone knows which files are crates'
# modules; every other changed file goes to the checkout's own oxfmt and
# Ruff, which skip what their settings ignore. A checkout without its npm
# packages or Python environment skips that formatter: the hook installs
# nothing. With no change, or outside a Git checkout, it ends at once.
# Unformatted files end the hook with status 2, which hands its message
# back to the agent to fix; a stop the hook already sent back ends quietly,
# so it never loops. It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

input=$(cat)
if grep -q '"stop_hook_active": *true' <<<"$input"; then
  exit 0
fi
root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$root"

changed=()
while IFS= read -r file; do
  [[ -n $file && -f $file ]] && changed+=("$file")
done < <(
  git diff --name-only HEAD 2>/dev/null
  git ls-files --others --exclude-standard
)
[[ ${#changed[@]} -gt 0 ]] || exit 0

workspace=false kennel=false
python=()
for file in "${changed[@]}"; do
  case $file in
    kennel/*.rs) kennel=true ;;
    *.rs) workspace=true ;;
    *.py) python+=("$file") ;;
  esac
done

failed=()
if $workspace && ! cargo fmt --all --check >/dev/null 2>&1; then
  failed+=("cargo fmt --all --check")
fi
if $kennel && ! (cd kennel && cargo fmt --check >/dev/null 2>&1); then
  failed+=("cd kennel && cargo fmt --check")
fi
if [[ -x node_modules/.bin/oxfmt ]] &&
  ! node_modules/.bin/oxfmt --check --no-error-on-unmatched-pattern "${changed[@]}" >/dev/null 2>&1; then
  failed+=("pnpm exec oxfmt --check")
fi
if [[ ${#python[@]} -gt 0 && -x .venv/bin/ruff ]] &&
  ! .venv/bin/ruff format --check --force-exclude "${python[@]}" >/dev/null 2>&1; then
  failed+=("uv run ruff format --check")
fi
if [[ ${#failed[@]} -gt 0 ]]; then
  {
    echo "The changed files are not formatted: run \`just fmt\`, then check again. Failed:"
    printf '  %s\n' "${failed[@]}"
  } >&2
  exit 2
fi
