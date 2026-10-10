#!/usr/bin/env bash
# Host (macOS arm64) side. Needs verdaccio on localhost:4873 and the Python index on localhost:8000.
#   ./macos.sh fixtures   lockfiles + npm caches produced on macOS (consumed by the Linux "maclock"/offline cells)
#   ./macos.sh cells      install into fresh temp dirs/venvs on the host; Gatekeeper/quarantine/codesign checks
set -uo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/out"; FX="$OUT/fixtures"; RES="$OUT/results"
REG=http://localhost:4873/
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
YARN_VERSION="${YARN_VERSION:-4.18.1}"
spec() { [ "$1" = native ] && echo "@automan-proto/native@0.1.0" || echo "@automan-proto/rpc@0.1.1"; }

fixtures() {
  rm -rf "$FX" && mkdir -p "$FX/locks" "$FX/tarballs"
  for pkg in native rpc; do
    for npmv in 11 10; do
      npmcmd=(npm); [ "$npmv" = 10 ] && npmcmd=(npx -y npm@10.8.2)
      d=$(mktemp -d); cache="$FX/maccache-$pkg-npm$npmv"
      (cd "$d" && echo '{"name":"cell","version":"0.0.0","private":true}' > package.json \
        && "${npmcmd[@]}" install -s --no-audit --no-fund --registry "$REG" --cache "$cache/npm" "$(spec "$pkg")")
      mkdir -p "$FX/locks/npm$npmv-$pkg" "$cache/proj"
      cp "$d/package.json" "$d/package-lock.json" "$FX/locks/npm$npmv-$pkg/"
      cp "$d/package.json" "$d/package-lock.json" "$cache/proj/"
      # The classic npm/cli#4828 sequence: lockfile deleted, node_modules kept, reinstall.
      (cd "$d" && rm package-lock.json && "${npmcmd[@]}" install -s --no-audit --no-fund --registry "$REG" --cache "$cache/npm")
      mkdir -p "$FX/locks/npm${npmv}regen-$pkg"; cp "$d/package.json" "$d/package-lock.json" "$FX/locks/npm${npmv}regen-$pkg/"
      rm -rf "$d"
    done
    d=$(mktemp -d)
    (cd "$d" && echo '{"name":"cell","version":"0.0.0","private":true}' > package.json && pnpm add -s --registry "$REG" "$(spec "$pkg")" >/dev/null)
    # pnpm >= 11 writes minimumReleaseAgeExclude entries for just-published versions into pnpm-workspace.yaml;
    # a user would commit it with the lockfile.
    mkdir -p "$FX/locks/pnpm-$pkg"; cp "$d/package.json" "$d/pnpm-lock.yaml" "$FX/locks/pnpm-$pkg/"
    mkdir -p "$FX/locks/pnpmnoexclude-$pkg"; cp "$d/package.json" "$d/pnpm-lock.yaml" "$FX/locks/pnpmnoexclude-$pkg/"
    [ -f "$d/pnpm-workspace.yaml" ] && cp "$d/pnpm-workspace.yaml" "$FX/locks/pnpm-$pkg/"; rm -rf "$d"
    d=$(mktemp -d)
    (cd "$d" && echo '{"name":"cell","version":"0.0.0","private":true}' > package.json \
      && printf 'npmRegistryServer: "%s"\nunsafeHttpWhitelist: ["localhost"]\nnodeLinker: node-modules\nenableGlobalCache: false\nenableTelemetry: false\nnpmMinimalAgeGate: 0\n' "$REG" > .yarnrc.yml \
      && corepack "yarn@$YARN_VERSION" add "$(spec "$pkg")" >/dev/null)
    mkdir -p "$FX/locks/yarn-$pkg"; cp "$d/package.json" "$d/yarn.lock" "$FX/locks/yarn-$pkg/"; rm -rf "$d"
    d=$(mktemp -d)
    (cd "$d" && echo '{"name":"cell","version":"0.0.0","private":true}' > package.json && printf '[install]\nregistry = "%s"\n' "$REG" > bunfig.toml && bun add "$(spec "$pkg")" >/dev/null 2>&1)
    mkdir -p "$FX/locks/bun-$pkg"; cp "$d/package.json" "$d/bun.lock" "$FX/locks/bun-$pkg/"; rm -rf "$d"
  done
  (cd "$OUT/npm/native" && npm pack -s --pack-destination "$FX/tarballs" >/dev/null && mv "$FX"/tarballs/automan-proto-native-*.tgz "$FX/tarballs/native-main.tgz")
  (cd "$OUT/npm/rpc-0.1.1" && npm pack -s --pack-destination "$FX/tarballs" >/dev/null && mv "$FX"/tarballs/automan-proto-rpc-*.tgz "$FX/tarballs/rpc-main.tgz")
  echo "fixtures: $(ls "$FX/locks" | tr '\n' ' ')"
}

secs_since() { python3 -c "import time,sys;print(f'{time.time()-float(sys.argv[1]):.1f}')" "$1"; }
now() { python3 -c 'import time;print(time.time())'; }
# rx bytes: no per-process counter on macOS; use the registry-side sizes (tarballs/wheels) instead.

gatekeeper() { # file -> one-line summary of quarantine + signature + assessment
  local f=$1 q sig
  q=$(xattr -p com.apple.quarantine "$f" 2>/dev/null || echo none)
  sig=$(codesign -dv "$f" 2>&1 | grep -E '^Signature=|flags=' | tr '\n' ' ')
  echo "quarantine=$q; $sig; spctl: $(spctl --assess --type execute "$f" 2>&1 | head -1)"
}

cell_npm() {
  local pkg=$1 C="mac-npm-$1"; local R="$RES/$C"; mkdir -p "$R"; local d; d=$(mktemp -d)
  cd "$d" && echo '{"name":"cell","version":"0.0.0","private":true}' > package.json
  local cache; cache=$(mktemp -d)
  local t0; t0=$(now)
  npm install --no-audit --no-fund --registry "$REG" --cache "$cache" "$(spec "$pkg")" > "$R/log.txt" 2>&1; local irc=$?
  local secs; secs=$(secs_since "$t0")
  if [ "$pkg" = native ]; then
    node -e "console.log(require('@automan-proto/native').hello())" > "$R/test.txt" 2>&1; trc=$?
    f=$(find "$d/node_modules" -name '*.node' | head -1)
  else
    node -e "require('@automan-proto/rpc').ping().then(r=>console.log(JSON.stringify(r)))" > "$R/test.txt" 2>&1; trc=$?
    f=$(find "$d/node_modules" -name engined -type f | head -1)
  fi
  { echo "install_rc=$irc"; echo "test_rc=$trc"; echo "secs=$secs"; echo "rx_bytes=n/a"
    echo "disk_kb=$(du -sk "$d/node_modules" | cut -f1)"
    echo "installed=$(ls "$d/node_modules/@automan-proto" | tr '\n' ' ')"
    echo "pm_version=npm $(npm --version)"; echo "runtime=$(node --version)"
    echo "gatekeeper=$(gatekeeper "$f")"; } > "$R/kv"
  cat "$R/test.txt" >> "$R/log.txt"; cd "$HERE"; rm -rf "$d" "$cache"
}

cell_py() {
  local pkg=$1 inst=$2 C="mac-$2-$1"; local R="$RES/$C"; mkdir -p "$R"; local d; d=$(mktemp -d)
  if [ "$inst" = pip ]; then uv venv -q --seed --python 3.13 "$d/venv"; else uv venv -q --python 3.13 "$d/venv"; fi
  local t0; t0=$(now)
  if [ "$inst" = pip ]; then
    "$d/venv/bin/python" -m pip install --no-cache-dir --disable-pip-version-check --index-url http://localhost:8000/full/simple/ "automan-$pkg" > "$R/log.txt" 2>&1; irc=$?
  else
    uv pip install --no-cache --python "$d/venv/bin/python" --index-url http://localhost:8000/full/simple/ "automan-$pkg" > "$R/log.txt" 2>&1; irc=$?
  fi
  local secs; secs=$(secs_since "$t0")
  if [ "$pkg" = native ]; then
    "$d/venv/bin/python" -c "import automan_native; print(automan_native.hello())" > "$R/test.txt" 2>&1; trc=$?
    f=$(find "$d/venv" -name 'automan_native*.so' | head -1)
  else
    "$d/venv/bin/python" -m automan_rpc > "$R/test.txt" 2>&1; trc=$?
    f="$d/venv/bin/engined"
  fi
  site=$("$d/venv/bin/python" -c 'import sysconfig;print(sysconfig.get_path("purelib"))')
  { echo "install_rc=$irc"; echo "test_rc=$trc"; echo "secs=$secs"; echo "rx_bytes=n/a"
    echo "disk_kb=$(du -skc "$site"/automan_* "$d/venv/bin/engined" 2>/dev/null | tail -1 | cut -f1)"
    echo "installed=$(sed -n 's/^Tag: //p' "$site"/automan_*.dist-info/WHEEL | tr '\n' ' ')"
    echo "pm_version=$([ "$inst" = pip ] && "$d/venv/bin/python" -m pip --version | cut -d' ' -f1-2 || uv --version)"
    echo "runtime=$("$d/venv/bin/python" --version)"
    echo "gatekeeper=$(gatekeeper "$f")"; } > "$R/kv"
  cat "$R/test.txt" >> "$R/log.txt"; rm -rf "$d"
}

cells() {
  for pkg in native rpc; do cell_npm "$pkg"; cell_py "$pkg" pip; cell_py "$pkg" uv; done
}

"$@"
