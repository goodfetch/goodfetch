#!/usr/bin/env bash
# The real OS services the tests meet (ADR 0059; TESTING.md, Storage, network
# and OS services):
#   setup            in CI, the one step every test job runs before ADR 0043's
#                    deny and its first test step: starts the services, then
#                    checks them (`xtask os-services check`). A service that
#                    does not come up fails the job; no test is skipped.
#                    `--without-service-manager` leaves the user's service
#                    manager out, for a caller whose tests do not include
#                    `service_manager::` (`just linux`, ci/linux.sh).
#   skipped <file>   for `just test`: in CI (`CI` set), and where the caller
#                    requires every service (`GOODFETCH_OS_SERVICES=required`,
#                    which ci/pre-merge.sh sets on Linux after running setup),
#                    the same check, failing on a missing service; on a
#                    local host otherwise, appends each
#                    missing service to <file> and prints the test-name
#                    patterns that skip the tests needing it (ci/tool.sh's rule).
#   addresses        prints the extra loopback addresses, one per line.
#
# What each OS gets:
#   - Linux: a D-Bus session bus and gnome-keyring-daemon's Secret Service,
#     its login keyring created and unlocked with a throwaway password drawn
#     here, as keyring-rs's CI does. Later steps find the bus through
#     DBUS_SESSION_BUS_ADDRESS: written to $GITHUB_ENV on a runner's own
#     system, and to $RUNTIME/env, which ci/vm/in-container.sh and
#     ci/pre-merge.sh read. XDG_RUNTIME_DIR, written beside it, stays the
#     user's own folder where systemd made one, for the user's manager.
#     The keyring keeps its files and control socket in $RUNTIME, apart from
#     any keyring of the user's login session. A setup run again while the
#     earlier session's Secret Service answers keeps that session.
#   - macOS: the extra loopback addresses as `lo0` aliases, which needs sudo.
#     The owner's machine's runner user has none: its aliases come from
#     ci/runner/loopback-aliases.sh, which the owner installs once (#1562).
#   - Linux also: `tc netem` on the loopback interface, which the Proving
#     ground's netem tests set (iproute2's `tc`, with CAP_NET_ADMIN): the
#     job's container gets the capability from ci/vm/start.sh; on a runner's
#     own system, `tc` gets it as a file capability, so the tests, run as the
#     runner's user, shape only their own network namespace's loopback.
#   - Linux also: Xvfb installed, which Collie's `virtual_display` tests
#     start for a headful browser on a display of their own.
#   - Windows and Linux: 127.0.0.0/8 answers already; the check confirms it.
#   - Linux also: user namespaces for Collie's PID 1 test, through a
#     root-owned copy of util-linux's `unshare` at $TEST_UNSHARE. Where
#     AppArmor restricts unprivileged user namespaces (Ubuntu 23.10 and
#     later), ci/apparmor/goodfetch-test-unshare gives them to that copy
#     only, as Chromium's docs do for Chrome (ADR 0049): no sysctl changes,
#     and /usr/bin/unshare stays restricted. The setup fails unless the copy
#     maps this user to root in a user namespace of its own. Where it
#     restricts them, ci/apparmor/goodfetch-chrome-for-testing gives them
#     to the pinned Chrome for Testing build too, whose sandbox needs them:
#     without it, every browser a test launched aborted at its start.
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

# The tests' copy of `unshare`; the AppArmor profile and Collie's PID 1 test
# name the same path.
TEST_UNSHARE=/usr/local/libexec/goodfetch-test-tools/unshare

# Installs the tests' `unshare`, with its AppArmor profile and Chrome for
# Testing's where AppArmor restricts unprivileged user namespaces, and checks
# that it maps this user to root in a user namespace of its own.
setup_user_namespaces() {
  local parser restricted=/proc/sys/kernel/apparmor_restrict_unprivileged_userns refused
  as_root install -d -m 0755 "$(dirname "$TEST_UNSHARE")"
  as_root install -m 0755 "$(command -v unshare)" "$TEST_UNSHARE"
  parser=$(PATH="$PATH:/usr/sbin:/sbin" command -v apparmor_parser || true)
  if [[ "$(cat "$restricted" 2>/dev/null)" == 1 && -n "$parser" ]]; then
    local profile
    for profile in goodfetch-test-unshare goodfetch-chrome-for-testing; do
      as_root install -m 0644 "$root/ci/apparmor/$profile" "/etc/apparmor.d/$profile"
      as_root "$parser" --replace "/etc/apparmor.d/$profile"
    done
  fi
  if ! refused=$("$TEST_UNSHARE" --user --map-root-user true 2>&1); then
    echo "error: $TEST_UNSHARE cannot map $(id -un) to root in a user namespace ($refused); where AppArmor restricts unprivileged user namespaces, ci/apparmor/goodfetch-test-unshare gives them to it, which needs apparmor_parser" >&2
    exit 1
  fi
  echo "$TEST_UNSHARE makes user namespaces."
}

# What a transient scope of the user's manager limited to 64 MiB reads as its
# own memory limit: the cgroup it runs in, then that cgroup's `memory.max`
# (67108864 when the limit holds) and its parent's controllers.
scope_memory_limit() {
  # The script expands in the scope's own shell, not here.
  # shellcheck disable=SC2016
  systemd-run --user --scope --quiet -p MemoryMax=64M -- sh -c '
    cgroup=$(sed -n "s/^0:://p" /proc/self/cgroup)
    echo "cgroup $cgroup"
    echo "memory.max $(cat "/sys/fs/cgroup$cgroup/memory.max" 2>&1)"
    echo "controllers $(cat "/sys/fs/cgroup$cgroup/../cgroup.controllers" 2>&1)"' 2>&1
}

# Collie's governor test runs the engine in a transient scope of the user's
# manager with a memory limit (crates/goodfetch-collie/tests/it/governor.rs).
# On 2026-10-23's hosted Release run the engine read no limit there on any
# Linux cell. When a scope's limit does not hold on a GitHub-hosted runner,
# the user's manager gets every controller delegated, as systemd's docs on
# delegation and rootless Podman's set it up, and the probe runs again. What
# each probe read is printed, and a limit that still does not hold is a
# warning naming it: the test then fails with its own message.
setup_memory_scopes() {
  local probe uid
  uid=$(id -u)
  probe=$(scope_memory_limit)
  if [[ "$probe" != *"memory.max 67108864"* && "${RUNNER_ENVIRONMENT:-}" == github-hosted ]]; then
    echo "A user scope's memory limit did not hold:"
    echo "  ${probe//$'\n'/$'\n'  }"
    as_root mkdir -p /etc/systemd/system/user@.service.d
    printf '[Service]\nDelegate=cpu cpuset io memory pids\n' |
      as_root tee /etc/systemd/system/user@.service.d/goodfetch-delegate.conf >/dev/null
    as_root systemctl daemon-reload
    as_root systemctl restart "user@$uid.service"
    probe=$(scope_memory_limit)
  fi
  echo "A user scope limited to 64 MiB reads:"
  echo "  ${probe//$'\n'/$'\n'  }"
  if [[ "$probe" != *"memory.max 67108864"* ]]; then
    echo "::warning::A transient scope of the user's manager does not limit its memory here, so Collie's governor test fails: ${probe//$'\n'/; }"
  fi
}

setup_linux() {
  local missing=()
  command -v dbus-daemon >/dev/null || missing+=(dbus-daemon)
  command -v gnome-keyring-daemon >/dev/null || missing+=(gnome-keyring)
  command -v secret-tool >/dev/null || missing+=(libsecret-tools)
  PATH="$PATH:/usr/sbin:/sbin" command -v tc >/dev/null || missing+=(iproute2)
  # Xvfb, the virtual display Collie's `virtual_display` tests start for a
  # headful browser.
  command -v Xvfb >/dev/null || missing+=(xvfb)
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
  setup_user_namespaces

  # A bus at a path, not an abstract socket, so the deny's network namespace
  # on a hosted runner reaches it too.
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$RUNTIME/bus"
  # A host that runs the gate again and again (the local gate on Linux,
  # ci/pre-merge.sh) keeps the session an earlier setup started while its
  # Secret Service answers, so a run still using it keeps it.
  if [[ -S "$RUNTIME/bus" ]] &&
    (cd "$root" && cargo run --quiet --locked -p xtask -- os-services check --secret-service) >/dev/null 2>&1; then
    echo "Kept the D-Bus session and Secret Service an earlier setup started."
  else
    # The earlier session's bus, if it still runs; its keyring daemon ends
    # with it.
    if [[ -f "$RUNTIME/dbus.pid" ]]; then kill "$(cat "$RUNTIME/dbus.pid")" 2>/dev/null || true; fi
    rm -rf "$RUNTIME"
    mkdir -p "$RUNTIME"
    chmod 700 "$RUNTIME"
    dbus-daemon --session --fork --address="$DBUS_SESSION_BUS_ADDRESS" --print-pid >"$RUNTIME/dbus.pid"

    # The password unlocks a keyring that lives as long as the session and
    # holds only the tests' own items; it is never written anywhere. The
    # keyring's files and control socket live in the session's folder, so
    # neither meets a keyring the user's login session started: its control
    # socket in the user's runtime folder, its keyrings in their home.
    local password
    password=$(od -An -N24 -tx1 /dev/urandom | tr -d ' \n')
    printf '%s' "$password" | XDG_DATA_HOME="$RUNTIME/data" gnome-keyring-daemon --unlock \
      --components=secrets --control-directory="$RUNTIME/keyring" --daemonize >/dev/null
    unset password
    echo "Started a D-Bus session and gnome-keyring's Secret Service, unlocked."
  fi

  # The user's systemd manager, kept running by lingering, when systemd
  # booted this host; `systemctl --user` reaches it through its own socket
  # in the user's runtime folder, `/run/user/<uid>/systemd/private`.
  if [[ -d /run/systemd/system ]] && command -v loginctl >/dev/null; then
    as_root loginctl enable-linger "$(id -un)" || true
  fi
  # So the tests keep that runtime folder when systemd made one, and only
  # the bus is the session's; elsewhere the session's folder stands in.
  local runtime_dir=$RUNTIME
  if [[ -d "/run/user/$(id -u)/systemd" ]]; then runtime_dir=/run/user/$(id -u); fi
  export XDG_RUNTIME_DIR=$runtime_dir
  if [[ -d "/run/user/$(id -u)/systemd" ]]; then setup_memory_scopes; fi
  {
    echo "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS"
    echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
  } >"$RUNTIME/env"
  if [[ -n "${GITHUB_ENV:-}" ]]; then cat "$RUNTIME/env" >>"$GITHUB_ENV"; fi
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
    if [[ "${2:-}" == --without-service-manager ]]; then
      kept=()
      for service in "${services[@]}"; do
        [[ "$service" == --service-manager ]] || kept+=("$service")
      done
      services=("${kept[@]}")
    elif [[ -n "${2:-}" ]]; then
      echo "usage: ci/os-services.sh setup [--without-service-manager]" >&2
      exit 2
    fi
    case "$(uname -s)" in
      Linux) setup_linux ;;
      Darwin) setup_macos ;;
    esac
    xtask_check
    echo "Every OS service the tests use answers."
    ;;
  skipped)
    file=${2:?usage: ci/os-services.sh skipped <file>}
    if [[ -n "${CI:-}" || "${GOODFETCH_OS_SERVICES:-}" == required ]]; then
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
    echo "usage: ci/os-services.sh setup [--without-service-manager]|skipped <file>|addresses" >&2
    exit 2
    ;;
esac
