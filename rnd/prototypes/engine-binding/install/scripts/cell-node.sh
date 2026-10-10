#!/bin/sh
# Runs INSIDE a fresh node/bun container (no Rust toolchain). POSIX sh: alpine has no bash.
# env: CELL PKG(native|rpc) PM(npm|pnpm|yarn|yarn-pnp|bun) MODE SPEC REG [LOCK=fixture dir for MODE=maclock] [AGEGATE=default]
# mounts: /fixtures (ro), /results (rw), optional /cache
set -u
R=/results/$CELL; mkdir -p "$R"; LOG=$R/log.txt; : > "$LOG"
mkdir -p /w && cd /w
host=$(echo "$REG" | sed -E 's#https?://([^/]+)/?.*#\1#')
echo "{\"name\":\"cell\",\"version\":\"0.0.0\",\"private\":true}" > package.json
printf 'registry=%s\nfund=false\naudit=false\n' "$REG" > .npmrc
case "$PM" in
  yarn|yarn-pnp)
    linker=node-modules; [ "$PM" = yarn-pnp ] && linker=pnp
    printf 'npmRegistryServer: "%s"\nunsafeHttpWhitelist: ["%s"]\nnodeLinker: %s\nenableGlobalCache: false\nenableTelemetry: false\n' \
      "$REG" "${host%%:*}" "$linker" > .yarnrc.yml
    # Yarn 4.18 quarantines versions younger than npmMinimalAgeGate (default 1440 min). Our packages were
    # published minutes ago, so cells disable the gate unless AGEGATE=default (measured separately).
    [ "${AGEGATE:-off}" = default ] || echo 'npmMinimalAgeGate: 0' >> .yarnrc.yml ;;
  bun) printf '[install]\nregistry = "%s"\n' "$REG" > bunfig.toml ;;
esac

case "$MODE" in
  maclock) cp /fixtures/locks/"${LOCK:-$PM-$PKG}"/* .
           # bun.lock pins full tarball URLs; point them at the same registry under its in-network name
           # (npm gets the same effect from --replace-registry-host=always).
           [ -f bun.lock ] && sed -i "s#http://localhost:4873/#$REG#g" bun.lock ;;
  offline-cache) cp /cache/proj/package.json /cache/proj/package-lock.json . ;;
esac

case "$PM:$MODE" in
  npm:default)        cmd="npm install $SPEC" ;;
  npm:ignore-scripts) cmd="npm install --ignore-scripts $SPEC" ;;
  npm:omit-optional)  cmd="npm install --omit=optional $SPEC" ;;
  npm:maclock) cmd="npm ci --replace-registry-host=always" ;;
  npm:offline-tarball) cmd="npm install --offline /fixtures/tarballs/$PKG-main.tgz" ;;
  npm:offline-cache)  cmd="npm ci --offline --cache /cache/npm --replace-registry-host=always" ;;
  npm:prime-cache)    cmd="npm install --cache /cache/npm $SPEC" ;;
  pnpm:default)       cmd="pnpm add $SPEC" ;;
  pnpm:ignore-scripts) cmd="pnpm add --ignore-scripts $SPEC" ;;
  pnpm:omit-optional) cmd="pnpm add $SPEC && rm -rf node_modules && pnpm install --no-optional" ;;
  pnpm:maclock)       cmd="pnpm install --frozen-lockfile" ;;
  yarn*:default)      cmd="yarn add $SPEC" ;;
  yarn*:maclock)      cmd="yarn install --immutable" ;;
  bun:default)        cmd="bun add $SPEC" ;;
  bun:ignore-scripts) cmd="bun add --ignore-scripts $SPEC" ;;
  bun:omit-optional)  cmd="bun add --omit=optional $SPEC" ;;
  bun:maclock)        cmd="bun install --frozen-lockfile" ;;
  *) echo "unsupported $PM:$MODE" > "$LOG"; echo "install_rc=99" > "$R/kv"; exit 0 ;;
esac

rx() { cat /sys/class/net/eth0/statistics/rx_bytes 2>/dev/null || echo 0; }
up() { cut -d' ' -f1 /proc/uptime; }
rx0=$(rx); t0=$(up)
echo "\$ $cmd" >> "$LOG"
sh -c "$cmd" >> "$LOG" 2>&1; irc=$?
t1=$(up); rx1=$(rx)
if [ "$MODE" = prime-cache ]; then mkdir -p /cache/proj && cp package.json package-lock.json /cache/proj/; fi

case "$PKG" in
  native) js="console.log(require('@automan-proto/native').hello())" ;;
  rpc) js="const s=require('@automan-proto/rpc');const p=s.enginedPath();console.log('binary',p,'mode',(require('fs').statSync(p).mode&0o777).toString(8));s.ping().then(r=>console.log(JSON.stringify(r)))" ;;
esac
case "$PM" in
  yarn-pnp) run="yarn node -e" ;;
  bun) run="bun -e" ;;
  *) run="node -e" ;;
esac
echo "--- first use: $run" >> "$LOG"
$run "$js" > "$R/test.txt" 2>&1; trc=$?
cat "$R/test.txt" >> "$LOG"

installed=$(find /w \( -name '*.node' -o -name engined -o -name '*.zip' \) 2>/dev/null | grep automan-proto \
  | grep -oE '(native|engined)-(darwin|linux)-[a-z0-9]+(-(gnu|musl))?' | sort -u | tr '\n' ' ')
disk=$(du -sk /w 2>/dev/null | cut -f1)
{
  echo "install_rc=$irc"; echo "test_rc=$trc"
  echo "secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.1f", b-a}')"
  echo "rx_bytes=$((rx1 - rx0))"; echo "disk_kb=$disk"; echo "installed=$installed"
  echo "pm_version=$($(echo "$PM" | sed 's/-pnp//') --version 2>/dev/null | head -1)"
  echo "runtime=$(node --version 2>/dev/null || bun --version)"
} > "$R/kv"
