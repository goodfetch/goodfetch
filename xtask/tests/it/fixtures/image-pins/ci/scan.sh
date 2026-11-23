#!/usr/bin/env bash
# Runs an image by digest; a comment may name ghcr.io/zaproxy/zaproxy:stable.
set -euo pipefail
image=$(sed -n 's/^digest = "\(.*\)"$/\1/p' ci/zap/image.toml)
podman run --rm "ghcr.io/zaproxy/zaproxy@$image" zap-baseline.py
podman pull ghcr.io/zaproxy/zaproxy:2.16.1@sha256:3333333333333333333333333333333333333333333333333333333333333333
docker run --rm --name probe -e KEY=value --volume "$PWD:/work" postgres:16@sha256:4444444444444444444444444444444444444444444444444444444444444444 psql
docker run --rm "$CI_IMAGE" true
docker run --detach --privileged --cgroupns private --tmpfs /run "$CI_IMAGE" /sbin/init
