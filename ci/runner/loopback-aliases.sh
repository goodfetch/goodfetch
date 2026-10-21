#!/usr/bin/env bash
# Adds the extra loopback addresses the tests use (ci/os-services.sh) as `lo0`
# aliases on the owner's machine, now and at every boot, through a launchd
# daemon: `sudo ci/runner/loopback-aliases.sh`, run once by the owner (#1562).
# The runner's standard user cannot add them, so the Release suite's macOS
# cells only check that they answer. Re-run it when ci/os-services.sh changes
# its count; `sudo ci/runner/loopback-aliases.sh remove` takes them away.
#
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

label=org.goodfetch.loopback-aliases
plist=/Library/LaunchDaemons/$label.plist
root=$(cd "$(dirname "$0")/../.." && pwd)

[[ "$(id -u)" -eq 0 ]] || {
  echo "error: run it with sudo: it adds lo0 aliases and a launchd daemon" >&2
  exit 2
}

if [[ "${1:-}" == remove ]]; then
  launchctl bootout system "$plist" 2>/dev/null || true
  rm -f "$plist"
  for address in $("$root/ci/os-services.sh" addresses); do
    ifconfig lo0 -alias "$address" 2>/dev/null || true
  done
  echo "Removed the lo0 aliases and $plist."
  exit 0
fi

addresses=$("$root/ci/os-services.sh" addresses)
commands=""
for address in $addresses; do
  commands="$commands/sbin/ifconfig lo0 alias $address up; "
done

cat >"$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$label</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string>
    <string>-c</string>
    <string>$commands</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
EOF
chown root:wheel "$plist"
chmod 644 "$plist"
# The aliases are added here too: `launchctl bootstrap` starts the daemon's
# first run without waiting for it, so the check below could run first.
for address in $addresses; do
  ifconfig lo0 alias "$address" up
done
launchctl bootout system "$plist" 2>/dev/null || true
launchctl bootstrap system "$plist"

for address in $addresses; do
  ifconfig lo0 | grep -q "inet $address " || {
    echo "error: $address is still not on lo0" >&2
    exit 1
  }
done
echo "lo0 answers on $(echo "$addresses" | head -n1) to $(echo "$addresses" | tail -n1), now and at every boot."
