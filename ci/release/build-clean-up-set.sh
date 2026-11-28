#!/usr/bin/env bash
# Builds a daily Clean-up rules set and OCI layout for one release line and
# format major (ADR 0047, issue 208).
#
# Emits under `<out-dir>`:
#   - `goodfetch-clean-up-rules-full.tar.xz`
#   - `goodfetch-clean-up-rules-embedded.txt`
#   - `oci/` (OCI image layout annotated with `dev.goodfetch.clean-up-rules.*`,
#     matching `xtask/src/release/clean_up_pin.rs`)
#
# Usage: ci/release/build-clean-up-set.sh <line> <format-major> <out-dir>
set -euo pipefail

line=${1:?usage: ci/release/build-clean-up-set.sh <line> <format-major> <out-dir>}
format_major=${2:?usage: ci/release/build-clean-up-set.sh <line> <format-major> <out-dir>}
out_dir=${3:?usage: ci/release/build-clean-up-set.sh <line> <format-major> <out-dir>}

stamp=$(date -u +%Y%m%d)
version="${stamp}"
seed_dir="crates/goodfetch-clean-up-rules/rules"

mkdir -p "$out_dir/oci/blobs/sha256"

# Assemble full archive and embedded rule text from the release line's rules.
tar -cJf "$out_dir/goodfetch-clean-up-rules-full.tar.xz" -C "$seed_dir" .
{
  printf '! goodfetch-clean-up-rules line=%s format_major=%s version=%s\n' "$line" "$format_major" "$version"
  for f in "$seed_dir"/*.txt; do
    [[ -f "$f" ]] || continue
    cat "$f"
    printf '\n'
  done
} > "$out_dir/goodfetch-clean-up-rules-embedded.txt"

full_sha=$(sha256sum "$out_dir/goodfetch-clean-up-rules-full.tar.xz" | awk '{print $1}')
full_size=$(wc -c < "$out_dir/goodfetch-clean-up-rules-full.tar.xz" | tr -d ' ')
emb_sha=$(sha256sum "$out_dir/goodfetch-clean-up-rules-embedded.txt" | awk '{print $1}')
emb_size=$(wc -c < "$out_dir/goodfetch-clean-up-rules-embedded.txt" | tr -d ' ')

cp "$out_dir/goodfetch-clean-up-rules-full.tar.xz" "$out_dir/oci/blobs/sha256/${full_sha}"
cp "$out_dir/goodfetch-clean-up-rules-embedded.txt" "$out_dir/oci/blobs/sha256/${emb_sha}"

# Empty OCI config blob (`{}`).
printf '{}' > "$out_dir/oci/config.json"
cfg_sha=$(sha256sum "$out_dir/oci/config.json" | awk '{print $1}')
cfg_size=$(wc -c < "$out_dir/oci/config.json" | tr -d ' ')
mv "$out_dir/oci/config.json" "$out_dir/oci/blobs/sha256/${cfg_sha}"

manifest=$(jq -cn \
  --arg cfg_digest "sha256:${cfg_sha}" \
  --argjson cfg_size "$cfg_size" \
  --arg full_digest "sha256:${full_sha}" \
  --argjson full_size "$full_size" \
  --arg emb_digest "sha256:${emb_sha}" \
  --argjson emb_size "$emb_size" \
  --arg line "$line" \
  --arg format_major "$format_major" \
  --arg version "$version" \
  '{
    schemaVersion: 2,
    mediaType: "application/vnd.oci.image.manifest.v1+json",
    artifactType: "application/vnd.goodfetch.clean-up-rules.v1",
    config: {
      mediaType: "application/vnd.oci.empty.v1+json",
      digest: $cfg_digest,
      size: $cfg_size
    },
    layers: [
      {
        mediaType: "application/x-xz",
        digest: $full_digest,
        size: $full_size,
        annotations: {
          "org.opencontainers.image.title": "goodfetch-clean-up-rules-full.tar.xz",
          "dev.goodfetch.clean-up-rules.part": "full"
        }
      },
      {
        mediaType: "text/plain; charset=utf-8",
        digest: $emb_digest,
        size: $emb_size,
        annotations: {
          "org.opencontainers.image.title": "goodfetch-clean-up-rules-embedded.txt",
          "dev.goodfetch.clean-up-rules.part": "embedded"
        }
      }
    ],
    annotations: {
      "dev.goodfetch.clean-up-rules.line": $line,
      "dev.goodfetch.clean-up-rules.format_major": $format_major,
      "dev.goodfetch.clean-up-rules.version": $version,
      "dev.goodfetch.clean-up-rules.gate": "pass"
    }
  }')

printf '%s' "$manifest" > "$out_dir/oci/manifest.json"
man_sha=$(sha256sum "$out_dir/oci/manifest.json" | awk '{print $1}')
man_size=$(wc -c < "$out_dir/oci/manifest.json" | tr -d ' ')
mv "$out_dir/oci/manifest.json" "$out_dir/oci/blobs/sha256/${man_sha}"

printf '{"imageLayoutVersion":"1.0.0"}\n' > "$out_dir/oci/oci-layout"
jq -cn \
  --arg digest "sha256:${man_sha}" \
  --argjson size "$man_size" \
  --arg tag "${line}-v${format_major}-${version}" \
  '{
    schemaVersion: 2,
    mediaType: "application/vnd.oci.image.index.v1+json",
    manifests: [
      {
        mediaType: "application/vnd.oci.image.manifest.v1+json",
        digest: $digest,
        size: $size,
        annotations: {
          "org.opencontainers.image.ref.name": $tag
        }
      }
    ]
  }' > "$out_dir/oci/index.json"

printf '%s\n' "$version" > "$out_dir/version.txt"
