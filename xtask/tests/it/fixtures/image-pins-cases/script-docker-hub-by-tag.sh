#!/usr/bin/env bash
# Runs a Docker Hub image by its short name and tag, with options before it.
set -euo pipefail
docker run --rm -v "$PWD:/work" --workdir /work postgres:16 \
  psql --version
