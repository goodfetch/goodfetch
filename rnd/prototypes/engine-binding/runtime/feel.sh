#!/bin/zsh
# "Feel" numbers: glue lines per binding, clean release build time per binding, artifact sizes.
# Clean builds go to fresh, uniquely named dirs under target/feel/ (gitignored build output).
set -eu
cd "${0:A:h}"
loc() { cat "$@" | grep -v '^\s*$' | grep -cv '^\s*\(//\|#\)' }
echo "### Glue code (non-blank, non-comment lines; engine-core is shared: $(loc engine-core/src/lib.rs) lines)"
echo
echo "| Binding | Native side | SDK side | Total |"
echo "| --- | ---: | ---: | ---: |"
n1=$(loc napi-addon/src/lib.rs); n2=$(loc sdk-ts/native.ts sdk-ts/types.ts); echo "| napi-rs (Node) | $n1 (napi-addon/src/lib.rs) | $n2 (sdk-ts/native.ts + types.ts) | $((n1 + n2)) |"
p1=$(loc pyo3-ext/src/lib.rs); p2=$(loc sdk-py/automan_sdk/native.py sdk-py/automan_sdk/__init__.py); echo "| PyO3 (Python) | $p1 (pyo3-ext/src/lib.rs) | $p2 (native.py + __init__.py) | $((p1 + p2)) |"
e1=$(loc engined/src/main.rs); t2=$(loc sdk-ts/rpc.ts sdk-ts/types.ts); y2=$(loc sdk-py/automan_sdk/rpc.py sdk-py/automan_sdk/__init__.py)
echo "| RPC (engined, both SDKs) | $e1 (engined/src/main.rs, incl. ~50 for inspect/check) | TS $t2 (rpc.ts + types.ts), Py $y2 (rpc.py + __init__.py) | $((e1 + t2 + y2)) |"
echo
stamp=$(date +%s)
SR=$(rustc --print sysroot)
echo "### Clean release build (no LTO), load at start: $(uptime | sed 's/.*averages: //')"
echo
echo "| Artifact | Clean build (s) | Size | Stripped |"
echo "| --- | ---: | ---: | ---: |"
t0=$(date +%s); cargo build -q --release -p engined --target-dir target/feel/$stamp-engined 2>/dev/null; t1=$(date +%s)
b=target/feel/$stamp-engined/release/engined; cp $b target/feel/$stamp-engined.stripped; strip target/feel/$stamp-engined.stripped
echo "| engined binary | $((t1 - t0)) | $(du -h $b | cut -f1) | $(du -h target/feel/$stamp-engined.stripped | cut -f1) |"
t0=$(date +%s); cargo build -q --release -p automan_napi --target-dir target/feel/$stamp-napi 2>/dev/null; t1=$(date +%s)
b=target/feel/$stamp-napi/release/libautoman_napi.dylib; cp $b target/feel/$stamp-napi.stripped; strip -x target/feel/$stamp-napi.stripped
echo "| napi-rs addon (.node) | $((t1 - t0)) | $(du -h $b | cut -f1) | $(du -h target/feel/$stamp-napi.stripped | cut -f1) |"
t0=$(date +%s)
SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.5.sdk RUSTFLAGS="-C link-arg=-fuse-ld=$SR/lib/rustlib/aarch64-apple-darwin/bin/gcc-ld/ld64.lld" \
  CARGO_TARGET_DIR=target/feel/$stamp-py uvx maturin build -q --release -m pyo3-ext/Cargo.toml -o target/feel/$stamp-wheel -i .venv/bin/python >/dev/null 2>&1
t1=$(date +%s)
w=$(ls target/feel/$stamp-wheel/*.whl)
echo "| PyO3 abi3 wheel | $((t1 - t0)) | $(du -h $w | cut -f1) (wheel), $(du -h target/feel/$stamp-py/release/libautoman_engine.dylib | cut -f1) (.so) | n/a |"
