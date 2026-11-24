#!/usr/bin/env bash
# Sets up, on the gate host, the user the nightly fuzzing's untrusted steps
# run as (TESTING.md, Fuzzing; `untrusted` in ci/fuzz-nightly.sh, #2564).
# Run it once as the job's user, the one whose crontab holds the entry of
# ci/fuzz-nightly.crontab, before that entry goes in; it needs sudo, and runs
# again with nothing to change. It spends nothing.
#
#   - The system user GOODFETCH_FUZZ_USER (goodfetch-fuzz), in a group of its
#     own, with no login shell, no password, no home and no sudo.
#   - /etc/sudoers.d/goodfetch-fuzz lets the job's user run commands as that
#     user, and as no other, without a password and without a terminal of
#     their own (`!use_pty`), so what a step runs stays in the step's process
#     group and the night's stop reaches it.
#   - An ACL lets that user pass through the job's user's home, never list
#     it: the toolchain (CARGO_HOME, RUSTUP_HOME) and the night's worktree,
#     under the main checkout, are readable by every user; gh's hosts.yml,
#     git's credentials and the main checkout's `.env` (scripts/env-setup.sh)
#     are the job's user's alone. The night grants that user what it writes
#     and checks, before it builds, that it can read none of those files.
#
# Linux only: the gate host is the one host that runs the nightly fuzzing.
set -euo pipefail

user=${GOODFETCH_FUZZ_USER:-goodfetch-fuzz}
job=$(id -un)
if [[ $(uname -s) != Linux ]]; then
  echo "error: the fuzzing's user is set up on the Linux gate host only" >&2
  exit 1
fi
if [[ $(id -u) == 0 ]]; then
  echo "error: run this as the job's user, not as root: the sudoers rule names the user who runs it" >&2
  exit 1
fi
for tool in setfacl useradd visudo; do
  if ! PATH="$PATH:/usr/sbin:/sbin" command -v "$tool" >/dev/null; then
    echo "error: $tool is missing (the acl and passwd packages hold setfacl and useradd)" >&2
    exit 1
  fi
done

if ! id -u "$user" >/dev/null 2>&1; then
  sudo -n "$(PATH="$PATH:/usr/sbin:/sbin" command -v useradd)" --system --user-group \
    --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin "$user"
  echo "Added the system user $user."
fi

rule=$(mktemp)
trap 'rm -f "$rule"' EXIT
cat >"$rule" <<EOF
# ci/fuzz-user.sh: $job runs the nightly fuzzing's untrusted steps as $user
# (TESTING.md, Fuzzing), in the step's own process group.
Defaults>$user !use_pty
$job ALL=($user) NOPASSWD: ALL
EOF
sudo -n "$(PATH="$PATH:/usr/sbin:/sbin" command -v visudo)" -cqf "$rule"
sudo -n install -m 0440 -o root -g root "$rule" /etc/sudoers.d/goodfetch-fuzz

setfacl -m "u:$user:x" "$HOME"

if ! sudo -n -u "$user" true; then
  echo "error: $job cannot run commands as $user through sudo" >&2
  exit 1
fi
echo "$job runs the untrusted steps as $user."
