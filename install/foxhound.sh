#!/usr/bin/env sh
# Goodfetch Foxhound installer (https://goodfetch.dev/install/foxhound.sh; ADRs 0090 and 0094, issue 144).
# Downloads the release archive and SHA256SUMS for the host platform, verifies
# the archive checksum before extracting, and installs `goodfetch-foxhound`.
set -eu

PRODUCT="foxhound"
BIN_NAME="goodfetch-${PRODUCT}"
REPO="${GOODFETCH_REPO:-goodfetch/goodfetch}"
INSTALL_DIR="${GOODFETCH_INSTALL_DIR:-${HOME}/.local/bin}"

detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Linux)
      case "$arch" in
        x86_64 | amd64) echo "x86_64-unknown-linux-musl" ;;
        aarch64 | arm64) echo "aarch64-unknown-linux-musl" ;;
        *)
          echo "error: unsupported Linux architecture: $arch" >&2
          exit 1
          ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        arm64 | aarch64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *)
          echo "error: unsupported macOS architecture: $arch" >&2
          exit 1
          ;;
      esac
      ;;
    *)
      echo "error: unsupported operating system: $os (use foxhound.ps1 on Windows)" >&2
      exit 1
      ;;
  esac
}

verify_sha256() {
  file=$1
  expected=$2
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$file" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$file" | awk '{print $1}')
  else
    echo "error: sha256sum or shasum is required to verify the archive" >&2
    exit 1
  fi
  if [ "$actual" != "$expected" ]; then
    echo "error: checksum mismatch for $file (expected $expected, got $actual)" >&2
    exit 1
  fi
}

target=$(detect_target)
archive="${BIN_NAME}-${target}.tar.xz"

if [ -n "${GOODFETCH_RELEASE_BASE:-}" ]; then
  base="${GOODFETCH_RELEASE_BASE%/}"
elif [ -n "${GOODFETCH_VERSION:-}" ]; then
  version="${GOODFETCH_VERSION#v}"
  base="https://github.com/${REPO}/releases/download/v${version}"
else
  base="https://github.com/${REPO}/releases/latest/download"
fi

scratch=$(mktemp -d)
cleanup() {
  rm -rf "$scratch"
}
trap cleanup EXIT INT TERM

echo "Downloading ${archive} from ${base}..." >&2
curl -fsSL "${base}/SHA256SUMS" -o "${scratch}/SHA256SUMS"
curl -fsSL "${base}/${archive}" -o "${scratch}/${archive}"

expected=$(awk -v f="$archive" '$2 == f || $2 == ("*" f) {print $1; exit}' "${scratch}/SHA256SUMS")
if [ -z "$expected" ]; then
  echo "error: ${archive} not listed in SHA256SUMS" >&2
  exit 1
fi
verify_sha256 "${scratch}/${archive}" "$expected"

tar -xJf "${scratch}/${archive}" -C "$scratch"
mkdir -p "$INSTALL_DIR"
install -m 0755 "${scratch}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"

echo "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}" >&2
echo "Optional: verify provenance with: gh attestation verify ${archive} --repo ${REPO}" >&2
