#!/usr/bin/env bash
# Pulls an image by its tag.
set -euo pipefail
podman pull ghcr.io/zaproxy/zaproxy:stable
