#!/usr/bin/env bash
# Checks `.gitignore` against the two ways it has broken a checkout. It fails
# when it reaches a crate's sources (#1761): a tracked Rust file under a
# `src/` folder, or the folder of the module it declares, where `mod tests;`
# in `src/a/target.rs` finds `src/a/target/tests.rs`; Git never lists an
# ignored file, so such a file passes every check in its author's checkout
# and is missing from the commit. It fails too when a build folder a tool
# makes in a Cargo package's folder, `<package>/target/`, is not ignored: a
# benchmark run there (gungraun under `cargo bench`) left the gate's tree
# unclean. It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

failed=0

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
  failed=1
fi

while IFS= read -r manifest; do
  folder=$(dirname "$manifest")
  # The root's `target` may be a link to a build folder elsewhere.
  if [[ $folder == . ]]; then build=target; else build=$folder/target/; fi
  if ! git check-ignore --no-index -q "$build"; then
    echo "error[unignored-build-folder] $build: .gitignore does not ignore this package's build folder, which a tool run in the package's folder writes"
    failed=1
  fi
done < <(git ls-files -- 'Cargo.toml' '*/Cargo.toml')

exit "$failed"
