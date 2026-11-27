//! OCI Image Layout builder and OCI Distribution pusher for first-party
//! Plugin multi-platform image indexes (ADRs 0033 and 0094; issue 141).

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use ureq::Agent;

const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.empty.v1+json";
const LAYER_MEDIA_TYPE: &str = "application/vnd.goodfetch.plugin.binary.v1+octet-stream";
const USER_AGENT: &str = "goodfetch-xtask (https://github.com/goodfetch/goodfetch)";

/// The six OCI platforms for a first-party Plugin multi-platform index
/// (Linux uses the static musl builds, ADR 0090).
const OCI_PLATFORMS: [(&str, &str, &str); 6] = [
    ("x86_64-unknown-linux-musl", "linux", "amd64"),
    ("aarch64-unknown-linux-musl", "linux", "arm64"),
    ("aarch64-apple-darwin", "darwin", "arm64"),
    ("x86_64-apple-darwin", "darwin", "amd64"),
    ("x86_64-pc-windows-msvc", "windows", "amd64"),
    ("aarch64-pc-windows-msvc", "windows", "arm64"),
];

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OciIndex {
    schema_version: u8,
    media_type: String,
    manifests: Vec<Descriptor>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OciManifest {
    schema_version: u8,
    media_type: String,
    config: Descriptor,
    layers: Vec<Descriptor>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    media_type: String,
    digest: String,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<Platform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotations: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Serialize, Deserialize)]
struct Platform {
    os: String,
    architecture: String,
}

fn write_blob(blobs_dir: &Path, bytes: &[u8]) -> Result<(String, u64)> {
    let hex = format!("{:x}", Sha256::digest(bytes));
    let digest = format!("sha256:{hex}");
    std::fs::write(blobs_dir.join(&hex), bytes)
        .with_context(|| format!("writing OCI blob {digest}"))?;
    Ok((digest, u64::try_from(bytes.len())?))
}

/// Builds a multi-platform OCI Image Layout for `plugin` from the binaries
/// present in `binaries/<target>/<plugin>[.exe]`.
pub(super) fn write_plugin_layout(
    plugin: &str,
    version: &str,
    binaries: &Path,
    out_dir: &Path,
) -> Result<()> {
    let blobs_dir = out_dir.join("blobs").join("sha256");
    std::fs::create_dir_all(&blobs_dir)
        .with_context(|| format!("creating {}", blobs_dir.display()))?;
    std::fs::write(
        out_dir.join("oci-layout"),
        "{\"imageLayoutVersion\":\"1.0.0\"}\n",
    )?;

    let empty_config = b"{}";
    let (config_digest, config_size) = write_blob(&blobs_dir, empty_config)?;

    let mut manifests = Vec::new();
    for (target, os, arch) in OCI_PLATFORMS {
        let filename = if os == "windows" {
            format!("{plugin}.exe")
        } else {
            plugin.to_owned()
        };
        let path = binaries.join(target).join(&filename);
        if !path.is_file() {
            continue;
        }
        let binary = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let (layer_digest, layer_size) = write_blob(&blobs_dir, &binary)?;

        let mut layer_annotations = std::collections::BTreeMap::new();
        layer_annotations.insert("org.opencontainers.image.title".to_owned(), filename);
        layer_annotations.insert(
            "org.opencontainers.image.version".to_owned(),
            version.to_owned(),
        );

        let manifest = OciManifest {
            schema_version: 2,
            media_type: MANIFEST_MEDIA_TYPE.to_owned(),
            config: Descriptor {
                media_type: CONFIG_MEDIA_TYPE.to_owned(),
                digest: config_digest.clone(),
                size: config_size,
                platform: None,
                annotations: None,
            },
            layers: vec![Descriptor {
                media_type: LAYER_MEDIA_TYPE.to_owned(),
                digest: layer_digest,
                size: layer_size,
                platform: None,
                annotations: Some(layer_annotations),
            }],
        };
        let manifest_bytes = serde_json::to_vec(&manifest)?;
        let (manifest_digest, manifest_size) = write_blob(&blobs_dir, &manifest_bytes)?;
        manifests.push(Descriptor {
            media_type: MANIFEST_MEDIA_TYPE.to_owned(),
            digest: manifest_digest,
            size: manifest_size,
            platform: Some(Platform {
                os: os.to_owned(),
                architecture: arch.to_owned(),
            }),
            annotations: None,
        });
    }

    let index = OciIndex {
        schema_version: 2,
        media_type: INDEX_MEDIA_TYPE.to_owned(),
        manifests,
    };
    let index_bytes = serde_json::to_vec_pretty(&index)?;
    std::fs::write(out_dir.join("index.json"), index_bytes)?;
    Ok(())
}

/// Pushes an OCI Image Layout (blobs, child manifests, and multi-platform
/// index) to `registry` under `repository:tag`.
pub(super) fn push_layout(
    layout: &Path,
    registry: &str,
    repository: &str,
    tag: &str,
    token: Option<&str>,
) -> Result<()> {
    let base = registry.trim_end_matches('/');
    let blobs_dir = layout.join("blobs").join("sha256");
    let index_bytes = std::fs::read(layout.join("index.json"))
        .with_context(|| format!("reading {}/index.json", layout.display()))?;
    let index: OciIndex = serde_json::from_slice(&index_bytes).context("parsing index.json")?;

    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();

    let read_blob = |digest: &str| -> Result<Vec<u8>> {
        let hex = digest
            .strip_prefix("sha256:")
            .with_context(|| format!("unsupported OCI digest {digest}"))?;
        let path = blobs_dir.join(hex);
        std::fs::read(&path).with_context(|| format!("reading OCI blob {}", path.display()))
    };

    for entry in &index.manifests {
        let manifest_bytes = read_blob(&entry.digest)?;
        let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)
            .with_context(|| format!("parsing child manifest {}", entry.digest))?;

        upload_blob(
            &agent,
            base,
            repository,
            &manifest.config.digest,
            &read_blob(&manifest.config.digest)?,
            token,
        )?;
        for layer in &manifest.layers {
            upload_blob(
                &agent,
                base,
                repository,
                &layer.digest,
                &read_blob(&layer.digest)?,
                token,
            )?;
        }
        put_manifest(
            &agent,
            base,
            repository,
            &entry.digest,
            &entry.media_type,
            &manifest_bytes,
            token,
        )?;
    }

    put_manifest(
        &agent,
        base,
        repository,
        tag,
        &index.media_type,
        &index_bytes,
        token,
    )?;
    Ok(())
}

fn upload_blob(
    agent: &Agent,
    base: &str,
    repository: &str,
    digest: &str,
    bytes: &[u8],
    token: Option<&str>,
) -> Result<()> {
    let head_url = format!("{base}/v2/{repository}/blobs/{digest}");
    let head = with_auth(agent.head(&head_url), token).call()?;
    if head.status() == 200 {
        return Ok(());
    }

    let start_url = format!("{base}/v2/{repository}/blobs/uploads/");
    let started = with_auth(agent.post(&start_url), token).send_empty()?;
    let status = started.status().as_u16();
    if status != 202 {
        bail!("{start_url} answered {status} starting upload of {digest}");
    }
    let location = started
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .with_context(|| format!("{start_url} returned no Location header"))?;
    let put_base = if location.starts_with("http://") || location.starts_with("https://") {
        location.to_owned()
    } else {
        format!("{base}{location}")
    };
    let sep = if put_base.contains('?') { '&' } else { '?' };
    let put_url = format!("{put_base}{sep}digest={digest}");
    let put = with_auth(agent.put(&put_url), token)
        .header("Content-Type", "application/octet-stream")
        .send(bytes)?;
    let put_status = put.status().as_u16();
    if put_status != 201 && put_status != 202 {
        bail!("{put_url} answered {put_status} uploading {digest}");
    }
    Ok(())
}

fn put_manifest(
    agent: &Agent,
    base: &str,
    repository: &str,
    reference: &str,
    media_type: &str,
    bytes: &[u8],
    token: Option<&str>,
) -> Result<()> {
    let url = format!("{base}/v2/{repository}/manifests/{reference}");
    let response = with_auth(agent.put(&url), token)
        .header("Content-Type", media_type)
        .send(bytes)?;
    let status = response.status().as_u16();
    if status != 201 && status != 200 {
        bail!("{url} answered {status} pushing manifest {reference}");
    }
    Ok(())
}

fn with_auth<B>(request: ureq::RequestBuilder<B>, token: Option<&str>) -> ureq::RequestBuilder<B> {
    let request = request.header("User-Agent", USER_AGENT);
    match token {
        Some(token) => request.header("Authorization", format!("Bearer {token}")),
        None => request,
    }
}
