# A Linux run of the lab on committed sources, for contract passes and Linux probes. It is a container in a VM on the
# developer machine, so the host class is local-dev at best (the reference gate requires bare metal): its numbers are never
# published. Build context: `git archive HEAD:lab`, so nothing uncommitted enters the image.
FROM node:25.5.0-bookworm

ARG RUST_VERSION=1.93.0
ARG GO_VERSION=1.27.1
ARG PNPM_VERSION=12.9.1

RUN apt-get update && apt-get install -y --no-install-recommends clang build-essential pkg-config cmake unzip git ca-certificates curl python3 \
  && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION}
ENV PATH=/root/.cargo/bin:/usr/local/go/bin:${PATH}
# Go from the official archive, checked against the checksum the same download page publishes.
RUN arch=$(dpkg --print-architecture) && file=go${GO_VERSION}.linux-${arch}.tar.gz \
  && curl -fsSLO https://go.dev/dl/${file} \
  && sha=$(curl -fsSL 'https://go.dev/dl/?mode=json&include=all' | python3 -c "import json,sys; print(next(f['sha256'] for r in json.load(sys.stdin) for f in r['files'] if f['filename']=='${file}'))") \
  && echo "${sha}  ${file}" | sha256sum -c - && tar -C /usr/local -xzf ${file} && rm ${file}
RUN npm install -g pnpm@${PNPM_VERSION}

WORKDIR /lab
COPY . .
ENV PLAYWRIGHT_BROWSERS_PATH=/lab/.browsers
RUN pnpm install --frozen-lockfile
RUN pnpm --filter @automan-lab/node-adapters exec playwright install --with-deps chromium
CMD ["node", "tools/lab.ts", "ci"]
