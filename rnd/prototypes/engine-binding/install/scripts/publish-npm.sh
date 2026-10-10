#!/usr/bin/env bash
# Assembles the npm packages for both strategies and publishes them to the local verdaccio.
#   @automan-proto/native            0.1.0  napi loader + optionalDependencies on native-<plat>
#   @automan-proto/native-<plat>     0.1.0  one .node each (os/cpu/libc)
#   @automan-proto/rpc               0.1.0  thin JS SDK, deps engined-<plat>@0.1.0 (no preferUnplugged)
#   @automan-proto/rpc               0.1.1  (latest) deps engined-<plat>@0.1.1 (preferUnplugged: true, as esbuild)
#   @automan-proto/rpc               0.2.0  (dist-tag "static") deps engined-linux-<arch>: ONE static-musl
#                                           binary per arch, no libc field, serving glibc and musl
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/out"
REG="${REG:-http://localhost:4873/}"
NPMRC="$OUT/npm/.npmrc"
rm -rf "$OUT/npm" && mkdir -p "$OUT/npm"
# verdaccio allows anonymous publish for our scope, but npm refuses to publish without some token.
printf 'registry=%s\n//localhost:4873/:_authToken=anonymous-local-prototype\n' "$REG" > "$NPMRC"
pub() { (cd "$1" && npm publish --userconfig "$NPMRC" --registry "$REG" ${2:-} >/dev/null 2>&1) && echo "   published $(cd "$1" && node -p 'require("./package.json").name+"@"+require("./package.json").version') ${2:-}"; }

meta() { # plat -> os cpu libc json fragments
  local plat=$1 os cpu libc=""
  os=${plat%%-*}; cpu=$(echo "$plat" | cut -d- -f2)
  case "$plat" in *-gnu) libc=',"libc":["glibc"]' ;; *-musl) libc=',"libc":["musl"]' ;; esac
  printf '"os":["%s"],"cpu":["%s"]%s' "$os" "$cpu" "$libc"
}

# ---- (A) native ----
optdeps=""
for f in "$OUT"/napi/automan-native.*.node; do
  plat=$(basename "$f" .node); plat=${plat#automan-native.}
  d="$OUT/npm/native-$plat"; mkdir -p "$d"; cp "$f" "$d/"
  cat > "$d/package.json" <<EOF
{"name":"@automan-proto/native-$plat","version":"0.1.0","main":"automan-native.$plat.node","files":["automan-native.$plat.node"],$(meta "$plat"),"license":"MIT"}
EOF
  pub "$d"
  optdeps="$optdeps\"@automan-proto/native-$plat\":\"0.1.0\","
done
d="$OUT/npm/native"; mkdir -p "$d"; cp "$OUT/napi-loader/index.js" "$OUT/napi-loader/index.d.ts" "$d/"
node -e '
const p=require(process.argv[1]); p.optionalDependencies=JSON.parse("{"+process.argv[2].replace(/,$/,"")+"}");
require("fs").writeFileSync(process.argv[3], JSON.stringify(p,null,2))' "$HERE/sdk/native-js/package.json" "$optdeps" "$d/package.json"
pub "$d"

# ---- (B) rpc ----
for ver in 0.1.0 0.1.1; do
  unplug=""; [ "$ver" = 0.1.1 ] && unplug=',"preferUnplugged":true'
  optdeps=""
  for f in "$OUT"/engined/engined-*; do
    plat=${f##*/engined-}
    d="$OUT/npm/engined-$plat-$ver"; mkdir -p "$d/bin"; cp "$f" "$d/bin/engined"; chmod 755 "$d/bin/engined"
    cat > "$d/package.json" <<EOF
{"name":"@automan-proto/engined-$plat","version":"$ver","files":["bin"],$(meta "$plat")$unplug,"license":"MIT"}
EOF
    pub "$d"
    optdeps="$optdeps\"@automan-proto/engined-$plat\":\"$ver\","
  done
  d="$OUT/npm/rpc-$ver"; mkdir -p "$d"; cp "$HERE/sdk/rpc-js/index.js" "$d/"
  cat > "$d/package.json" <<EOF
{"name":"@automan-proto/rpc","version":"$ver","main":"index.js","files":["index.js"],"optionalDependencies":{${optdeps%,}},"license":"MIT"}
EOF
  pub "$d"
done

optdeps='"@automan-proto/engined-darwin-arm64":"0.1.1",'
for arch in arm64 x64; do
  f="$OUT/engined/engined-linux-$arch-musl"; [ -f "$f" ] || continue
  d="$OUT/npm/engined-linux-$arch-static"; mkdir -p "$d/bin"; cp "$f" "$d/bin/engined"; chmod 755 "$d/bin/engined"
  cat > "$d/package.json" <<EOF
{"name":"@automan-proto/engined-linux-$arch","version":"0.2.0","files":["bin"],"os":["linux"],"cpu":["$arch"],"preferUnplugged":true,"license":"MIT"}
EOF
  pub "$d"
  optdeps="$optdeps\"@automan-proto/engined-linux-$arch\":\"0.2.0\","
done
d="$OUT/npm/rpc-0.2.0"; mkdir -p "$d"; cp "$HERE/sdk/rpc-js/index.js" "$d/"
cat > "$d/package.json" <<EOF
{"name":"@automan-proto/rpc","version":"0.2.0","main":"index.js","files":["index.js"],"optionalDependencies":{${optdeps%,}},"license":"MIT"}
EOF
pub "$d" "--tag static"

# tarball sizes = what a registry install downloads per package
for d in "$OUT"/npm/*/; do (cd "$d" && npm pack --dry-run --json 2>/dev/null | node -e '
let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const [p]=JSON.parse(s);console.log(p.name+"@"+p.version+"\t"+p.size+"\t"+p.unpackedSize)})'); done > "$OUT/npm-sizes.tsv"
