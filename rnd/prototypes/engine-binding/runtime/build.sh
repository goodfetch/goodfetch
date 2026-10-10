#!/bin/zsh
# Builds engined, the napi-rs addon and the PyO3 wheel (installed into .venv).
set -eu
cd "${0:A:h}"
cargo build --release --target-dir target -p engined -p automan_napi
cp target/release/libautoman_napi.dylib napi-addon/automan_napi.node
[[ -x .venv/bin/python ]] || uv venv -q --python 3.13 .venv
# Workaround (macOS 27 beta toolchain): Apple ld-27037 left the PyO3 dylib's LINKEDIT string
# pool 4-byte aligned (odd indirect-symbol count) and dyld refused to load it ("mis-aligned
# LINKEDIT string pool"). Link this one with rust-lld against the 15.5 SDK instead.
SR=$(rustc --print sysroot)
rm -f dist/automan_engine-*.whl
SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.5.sdk \
RUSTFLAGS="-C link-arg=-fuse-ld=$SR/lib/rustlib/aarch64-apple-darwin/bin/gcc-ld/ld64.lld" \
CARGO_TARGET_DIR=target/py uvx maturin build --release -m pyo3-ext/Cargo.toml -o dist -i .venv/bin/python
uv pip install -q --python .venv/bin/python --force-reinstall dist/automan_engine-*.whl
echo "build ok"
