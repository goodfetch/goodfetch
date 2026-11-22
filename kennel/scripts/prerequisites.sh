# shellcheck shell=bash
# What the Kennel's Clippy needs beyond its toolchain, and the rule for what
# is missing, sourced by `just lint`'s Clippy on the crates the workspace
# excludes (ci/clippy-excluded.sh), the one place that lints the Kennel
# (#3217): the probes and their reasons live here alone.
#
#   kennel_cross                       the macOS target a host other than macOS
#                                      lints the Kennel for (#1915)
#   kennel_cross_missing               prints why that target cannot be
#                                      linted for, or nothing
#   kennel_linux_libraries_missing     prints why the graphical crate,
#                                      goodfetch-kennel-app, cannot be built
#                                      for this Linux host, or nothing
#   kennel_skip <root> <label> <what>  lists <what>, the part left out and
#                                      why, as skipped in
#                                      <root>/target/skipped.txt; exits 1 in
#                                      CI instead, which runs every part
#                                      (ADR 0061)

kennel_cross=aarch64-apple-darwin

kennel_cross_missing() {
  if ! rustup target list --installed | grep -qx "$kennel_cross"; then
    echo "the Rust target $kennel_cross is not installed (rustup target add $kennel_cross)"
  fi
}

# GTK 3 and libdbus, found by pkg-config: AppIndicator's library is loaded
# at run time, so it needs none.
kennel_linux_libraries_missing() {
  local absent=() library
  for library in gtk+-3.0 dbus-1; do
    pkg-config --exists "$library" || absent+=("$library")
  done
  if [[ ${#absent[@]} -gt 0 ]]; then
    echo "pkg-config finds no ${absent[*]}: GTK 3 (gtk+-3.0) is what its Tray builds on, libdbus (dbus-1) what its desktop notices send through (Debian and Ubuntu: apt install libgtk-3-dev libdbus-1-dev)"
  fi
}

kennel_skip() {
  local root=$1 label=$2 what=$3
  if [[ -n "${CI:-}" ]]; then
    echo "error: $label: $what, and CI runs every part" >&2
    exit 1
  fi
  mkdir -p "$root/target"
  echo "$label: skipped, $what" >>"$root/target/skipped.txt"
  echo "skipped: $label ($what)" >&2
}
