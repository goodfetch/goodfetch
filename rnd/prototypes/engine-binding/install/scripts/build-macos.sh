#!/usr/bin/env bash
# Builds the macOS arm64 artifacts on the host and generates the napi-rs JS loader.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$HERE/out"
NAPI_CLI_VERSION=3.10.8
mkdir -p "$OUT/napi" "$OUT/engined" "$OUT/wheels/native" "$OUT/wheels/rpc" "$OUT/tools" "$OUT/napi-loader"
t0=$(date +%s)

# napi-rs CLI (only to build + generate the platform-switching loader index.js).
if [ ! -x "$OUT/tools/node_modules/.bin/napi" ]; then
  (cd "$OUT/tools" && npm init -y >/dev/null && npm i --silent "@napi-rs/cli@$NAPI_CLI_VERSION")
fi
cp "$HERE/sdk/native-js/package.json" "$OUT/napi-loader/package.json"
s=$(date +%s)
"$OUT/tools/node_modules/.bin/napi" build --platform --release \
  --manifest-path "$HERE/src/native-node/Cargo.toml" \
  --package-json-path "$OUT/napi-loader/package.json" \
  --output-dir "$OUT/napi-loader" --js index.js --dts index.d.ts
mv "$OUT/napi-loader/automan-native.darwin-arm64.node" "$OUT/napi/"
echo "   napi .node (darwin-arm64) + loader built in $(( $(date +%s) - s ))s"

s=$(date +%s)
cargo build -q --release --manifest-path "$HERE/src/Cargo.toml" -p engined
cp "$HERE/src/target/release/engined" "$OUT/engined/engined-darwin-arm64"
echo "   engined (darwin-arm64) built in $(( $(date +%s) - s ))s"

s=$(date +%s)
uvx "maturin==1.15.*" build -q --release -m "$HERE/src/native-py/Cargo.toml" --features abi3 -o "$OUT/wheels/native"
for py in 3.13 3.14t; do
  interp="$(uv python find "$py" 2>/dev/null || (uv python install -q "$py" && uv python find "$py"))"
  uvx "maturin==1.15.*" build -q --release -m "$HERE/src/native-py/Cargo.toml" -i "$interp" -o "$OUT/wheels/native"
done
uvx "maturin==1.15.*" build -q --release -m "$HERE/src/engined/Cargo.toml" -o "$OUT/wheels/rpc"
echo "   macOS wheels built in $(( $(date +%s) - s ))s"
echo "macos build total $(( $(date +%s) - t0 ))s"
