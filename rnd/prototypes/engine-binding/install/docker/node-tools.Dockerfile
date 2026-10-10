# Fresh Node image + the package managers under test, pre-fetched so cells measure only our install.
# No Rust toolchain, no compiler.
ARG BASE=node:24-slim
FROM ${BASE}
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0 COREPACK_HOME=/opt/corepack
RUN npm i -g corepack@latest >/dev/null 2>&1 || true; \
    corepack enable && corepack install -g pnpm@12.12.0 yarn@4.18.1 && pnpm --version && yarn --version && chmod -R a+rwX /opt/corepack
