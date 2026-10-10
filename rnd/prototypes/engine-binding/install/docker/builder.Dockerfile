# Linux build image: Rust 1.93 + maturin + cargo-zigbuild + zig.
# zig is what real napi-rs / maturin release pipelines use to target old glibc (manylinux2014 = glibc 2.17)
# and musl from one image, without per-target sysroots or amd64 emulation.
FROM rust:1.93-slim-bookworm
COPY --from=ghcr.io/astral-sh/uv:0.9 /uv /usr/local/bin/uv
ENV UV_PYTHON_INSTALL_DIR=/opt/py VIRTUAL_ENV=/opt/tools PATH=/opt/tools/bin:$PATH
RUN uv venv --python 3.13 /opt/tools \
 && uv pip install --python /opt/tools/bin/python "maturin==1.15.*" cargo-zigbuild ziglang \
 && printf '#!/bin/sh\nexec /opt/tools/bin/python -m ziglang "$@"\n' > /usr/local/bin/zig && chmod +x /usr/local/bin/zig \
 && rustup target add aarch64-unknown-linux-gnu aarch64-unknown-linux-musl x86_64-unknown-linux-gnu x86_64-unknown-linux-musl
WORKDIR /src
