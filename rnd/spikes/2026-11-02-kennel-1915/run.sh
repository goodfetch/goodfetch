#!/bin/sh
# THROWAWAY SPIKE for goodfetch/goodfetch#1915.
#   ./run.sh terminal   the bare binary, ad hoc signed, Accessory policy set in code only
#   ./run.sh app        the same binary inside an ad hoc signed .app, run directly from this shell
#   ./run.sh app-open   the same .app launched through LaunchServices (`open`), as Finder would;
#                       its output goes to target/app-open.log, which this script follows
# Each runs until the menu's Quit or Ctrl-C.
set -eu
cd "$(dirname "$0")"
mode=${1:-app}

cargo build --release
bin=target/release/kennel-spike
echo "binary size (stripped, LTO, tray and window code, glow and wgpu): $(stat -f %z "$bin") bytes"

app=target/KennelSpike.app

build_app() {
    rm -rf "$app.new"
    mkdir -p "$app.new/Contents/MacOS"
    cp "$bin" "$app.new/Contents/MacOS/kennel-spike"
    cp Info.plist "$app.new/Contents/Info.plist"
    codesign --sign - --force "$app.new"
    # Install by rename, never by overwriting a signed binary in place.
    rm -rf "$app"
    mv "$app.new" "$app"
    codesign --display --verbose=2 "$app" 2>&1 | grep -E 'Identifier|Signature'
}

case $mode in
terminal)
    codesign --sign - --force "$bin"
    exec "$bin"
    ;;
app)
    build_app
    exec "$app/Contents/MacOS/kennel-spike"
    ;;
app-open)
    build_app
    log="$PWD/target/app-open.log"
    : >"$log"
    tail -f "$log" &
    tail_pid=$!
    open -n -W --stdout "$log" --stderr "$log" "$app" || true
    kill "$tail_pid" 2>/dev/null || true
    ;;
*)
    echo "usage: ./run.sh terminal|app|app-open" >&2
    exit 2
    ;;
esac
