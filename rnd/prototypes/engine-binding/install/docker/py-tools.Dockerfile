# Fresh Python image + uv (and poetry when WITH_POETRY=1). No Rust toolchain, no compiler.
# BASE=debian:bookworm-slim + FT=3.14t gives a free-threaded CPython installed by uv.
ARG BASE=python:3.13-slim
FROM ${BASE}
COPY --from=ghcr.io/astral-sh/uv:0.9 /uv /usr/local/bin/uv
ARG WITH_POETRY=0
ARG FT=""
ENV UV_PYTHON_INSTALL_DIR=/opt/py
RUN if [ -n "$FT" ]; then uv python install "$FT" && ln -s "$(uv python find "$FT")" /usr/local/bin/python3; fi; \
    if [ "$WITH_POETRY" = 1 ]; then UV_TOOL_BIN_DIR=/usr/local/bin uv tool install "poetry>=2" && poetry --version; fi; \
    python3 --version
