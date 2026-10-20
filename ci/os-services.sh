#!/usr/bin/env bash
# The real OS services the tests meet (ADR 0059; TESTING.md, Storage, network
# and OS services):
#   setup            in CI, the one step every test job runs before ADR 0043's
#                    deny and its first test step: starts the services, then
#                    checks them (`xtask os-services check`). A service that
#                    does not come up fails the job; no test is skipped.
#   skipped <file>   for `just test`: in CI (`CI` set) the same check, failing
#                    on a missing service; on a local host, appends each
#                    missing service to <file> and prints the test-name
#                    patterns that skip the tests needing it (ci/tool.sh's rule).
#   addresses        prints the extra loopback addresses, one per line.
#
# What each OS gets:
#   - Linux: a D-Bus session bus and gnome-keyring-daemon's Secret Service,
#     its login keyring created and unlocked with a throwaway password drawn
#     here, as keyring-rs's CI does. Later steps find the bus through
#     DBUS_SESSION_BUS_ADDRESS: written to $GITHUB_ENV on a runner's own
#     system, and to $RUNTIME/env, which ci/vm/in-container.sh reads, in the
#     job's container.
#   - macOS: the extra loopback addresses as `lo0` aliases, which needs sudo.
#     The owner's machine's runner user has none: its aliases come from
#     ci/runner/loopback-aliases.sh, which the owner installs once (#1562).
#   - Linux also: `tc netem` on the loopback interface, which the Proving
#     ground's netem tests set (iproute2's `tc`, with CAP_NET_ADMIN): the
#     job's container gets the capability from ci/vm/start.sh; on a runner's
#     own system, `tc` gets it as a file capability, so the tests, run as the
#     runner's user, shape only their own network namespace's loopback.
#   - Windows and Linux: 127.0.0.0/8 answers already; the check confirms it.
#   - Linux also: systemd's user manager, which `daemon install`'s tests
#     register services with for real in the Pre-merge suite (spec #213). A
#     host booted with systemd keeps the user's manager running through
#     lingering; a container that does not boot systemd has none, and the
#     check fails. On macOS and Windows those tests run in the Release suite
#     only, so a local `just test` of another suite skips them.
#
# It keeps to bash 3.2, which stock macOS runs on the owner's machine.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)

# The extra loopback addresses the tests may use: 127.0.0.2 upward, this many.
EXTRA_LOOPBACK_COUNT=8
addresses=()
for ((i = 2; i <= EXTRA_LOOPBACK_COUNT + 1; i++)); do addresses+=("127.0.0.$i"); done

# Where the Linux session's bus, keyring and environment file live.
RUNTIME=${RUNNER_TEMP:-/tmp}/goodfetch-os-services

services=(--loopback "${addresses[@]}")
[[ "$(uname -s)" == Linux ]] && services+=(--secret-service --netem --service-manager)

xtask_check() {
  (cd "$root" && cargo run --quiet --locked -p xtask -- os-services check "${services[@]}" "$@")
}

# Runs a command as root: directly in a container, through sudo on a runner.
as_root() {
  if [[ "$(id -u)" -eq 0 ]]; then "$@"; else sudo -n "$@"; fi
}

setup_linux() {
  local missing=()
  command -v dbus-daemon >/dev/null || missing+=(dbus-daemon)
  command -v gnome-keyring-daemon >/dev/null || missing+=(gnome-keyring)
  command -v secret-tool >/dev/null || missing+=(libsecret-tools)
  PATH="$PATH:/usr/sbin:/sbin" command -v tc >/dev/null || missing+=(iproute2)
  if [[ "$(id -u)" -ne 0 ]]; then
    PATH="$PATH:/usr/sbin:/sbin" command -v setcap >/dev/null || missing+=(libcap2-bin)
  fi
  if [[ ${#missing[@]} -gt 0 ]]; then
    as_root apt-get update -qq
    as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "${missing[@]}"
  fi
  if [[ "$(id -u)" -ne 0 ]]; then
    as_root setcap cap_net_admin+ep "$(PATH="$PATH:/usr/sbin:/sbin" command -v tc)"
  fi

  rm -rf "$RUNTIME"
  mkdir -p "$RUNTIME"
  chmod 700 "$RUNTIME"
  export XDG_RUNTIME_DIR=$RUNTIME
  # A bus at a path, not an abstract socket, so the deny's network namespace
  # on a hosted runner reaches it too.
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$RUNTIME/bus"
  dbus-daemon --session --fork --address="$DBUS_SESSION_BUS_ADDRESS"

  # The password unlocks a keyring that lives as long as the job and holds
  # only the tests' own items; it is never written anywhere.
  local password
  password=$(od -An -N24 -tx1 /dev/urandom | tr -d ' \n')
  printf '%s' "$password" | gnome-keyring-daemon --unlock --components=secrets --daemonize >/dev/null
  unset password

  {
    echo "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS"
    echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
  } >"$RUNTIME/env"
  if [[ -n "${GITHUB_ENV:-}" ]]; then cat "$RUNTIME/env" >>"$GITHUB_ENV"; fi
  echo "Started a D-Bus session and gnome-keyring's Secret Service, unlocked."

  # The user's systemd manager, kept running by lingering, when systemd
  # booted this host; `systemctl --user` reaches it through its own bus.
  if [[ -d /run/systemd/system ]] && command -v loginctl >/dev/null; then
    as_root loginctl enable-linger "$(id -un)" || true
  fi
}

setup_macos() {
  local address
  for address in "${addresses[@]}"; do
    ifconfig lo0 | grep -q "inet $address " && continue
    as_root ifconfig lo0 alias "$address" up || {
      echo "error: $address is not on lo0, and this user cannot add it; on the owner's machine, the owner runs \`sudo ci/runner/loopback-aliases.sh\` once (#1562)" >&2
      exit 1
    }
  done
  echo "lo0 answers on ${addresses[0]} to ${addresses[${#addresses[@]} - 1]}."
}

case "${1:-}" in
  setup)
    case "$(uname -s)" in
      Linux) setup_linux ;;
      Darwin) setup_macos ;;
    esac
    xtask_check
    echo "Every OS service the tests use answers."
    ;;
  skipped)
    file=${2:?usage: ci/os-services.sh skipped <file>}
    if [[ -n "${CI:-}" ]]; then
      xtask_check >&2
    else
      mkdir -p "$(dirname "$file")"
      if [[ "$(uname -s)" != Linux ]]; then
        services+=(--service-manager --suite "${GOODFETCH_TEST_SUITE:-quick}")
      fi
      xtask_check --skipped "$file"
    fi
    ;;
  addresses)
    printf '%s\n' "${addresses[@]}"
    ;;
  *)
    echo "usage: ci/os-services.sh setup|skipped <file>|addresses" >&2
    exit 2
    ;;
esac
