//! OCI Image Layout builder and OCI Distribution pusher for first-party
//! Plugin multi-platform image indexes (ADRs 0033 and 0094; issue 141).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use ureq::Agent;

const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.empty.v1+json";
const LAYER_MEDIA_TYPE: &str = "application/vnd.goodfetch.plugin.binary.v1+octet-stream";
const HELM_CONFIG_MEDIA_TYPE: &str = "application/vnd.cncf.helm.config.v1+json";
const HELM_CHART_LAYER_MEDIA_TYPE: &str = "application/vnd.cncf.helm.chart.content.v1.tar+gzip";
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

/// Packages one Product's Helm chart from the single shared chart source
/// (`packaging/helm/goodfetch/`) into a CNCF Helm OCI Image Layout (`out_dir`),
/// differing between Collie and Foxhound only by the chart name and the image
/// repository it runs (ADRs 0049 and 0094; issue 202).
pub(super) fn build_helm_chart_layout(
    chart_src: &Path,
    product_key: &str,
    version: &str,
    out_dir: &Path,
) -> Result<()> {
    let blobs_dir = out_dir.join("blobs").join("sha256");
    std::fs::create_dir_all(&blobs_dir)
        .with_context(|| format!("creating {}", blobs_dir.display()))?;

    let chart_yaml_raw = std::fs::read_to_string(chart_src.join("Chart.yaml"))
        .with_context(|| format!("reading {}/Chart.yaml", chart_src.display()))?;
    let values_yaml_raw = std::fs::read_to_string(chart_src.join("values.yaml"))
        .with_context(|| format!("reading {}/values.yaml", chart_src.display()))?;

    let mut chart_yaml = String::new();
    for line in chart_yaml_raw.lines() {
        if line.starts_with("name:") {
            let _ = writeln!(chart_yaml, "name: {product_key}");
        } else if line.starts_with("version:") {
            let _ = writeln!(chart_yaml, "version: {version}");
        } else if line.starts_with("appVersion:") {
            let _ = writeln!(chart_yaml, "appVersion: \"{version}\"");
        } else {
            chart_yaml.push_str(line);
            chart_yaml.push('\n');
        }
    }
    let values_yaml = values_yaml_raw.replace(
        "repository: ghcr.io/goodfetch/collie",
        &format!("repository: ghcr.io/goodfetch/{product_key}"),
    );
    let tar_gz = build_helm_chart_tgz(chart_src, product_key, &chart_yaml, &values_yaml)?;

    let config_json = serde_json::json!({
        "name": product_key,
        "version": version,
        "appVersion": version,
        "apiVersion": "v2",
        "type": "application",
        "description": format!("Goodfetch {product_key} Helm chart (ADRs 0049, 0094)"),
    });
    let config_bytes = serde_json::to_vec(&config_json)?;
    let (config_digest, config_size) = write_blob(&blobs_dir, &config_bytes)?;
    let (layer_digest, layer_size) = write_blob(&blobs_dir, &tar_gz)?;

    let mut annotations = std::collections::BTreeMap::new();
    annotations.insert(
        "org.opencontainers.image.title".to_owned(),
        format!("{product_key}-{version}.tgz"),
    );
    annotations.insert(
        "org.opencontainers.image.version".to_owned(),
        version.to_owned(),
    );

    let manifest = OciManifest {
        schema_version: 2,
        media_type: MANIFEST_MEDIA_TYPE.to_owned(),
        config: Descriptor {
            media_type: HELM_CONFIG_MEDIA_TYPE.to_owned(),
            digest: config_digest,
            size: config_size,
            platform: None,
            annotations: None,
        },
        layers: vec![Descriptor {
            media_type: HELM_CHART_LAYER_MEDIA_TYPE.to_owned(),
            digest: layer_digest,
            size: layer_size,
            platform: None,
            annotations: Some(annotations),
        }],
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let (manifest_digest, manifest_size) = write_blob(&blobs_dir, &manifest_bytes)?;

    let index = OciIndex {
        schema_version: 2,
        media_type: INDEX_MEDIA_TYPE.to_owned(),
        manifests: vec![Descriptor {
            media_type: MANIFEST_MEDIA_TYPE.to_owned(),
            digest: manifest_digest,
            size: manifest_size,
            platform: None,
            annotations: None,
        }],
    };
    std::fs::write(
        out_dir.join("oci-layout"),
        b"{\"imageLayoutVersion\":\"1.0.0\"}\n",
    )?;
    std::fs::write(
        out_dir.join("index.json"),
        serde_json::to_vec_pretty(&index)?,
    )?;
    Ok(())
}

fn build_helm_chart_tgz(
    chart_src: &Path,
    product_key: &str,
    chart_yaml: &str,
    values_yaml: &str,
) -> Result<Vec<u8>> {
    let mut tar_gz = Vec::new();
    let encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(&mut tar_gz, flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    append_tar_bytes(
        &mut tar,
        &format!("{product_key}/Chart.yaml"),
        chart_yaml.as_bytes(),
    )?;
    append_tar_bytes(
        &mut tar,
        &format!("{product_key}/values.yaml"),
        values_yaml.as_bytes(),
    )?;
    let templates_dir = chart_src.join("templates");
    if templates_dir.is_dir() {
        let mut entries: Vec<_> =
            std::fs::read_dir(&templates_dir)?.collect::<std::result::Result<Vec<_>, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let bytes = std::fs::read(&path)?;
                append_tar_bytes(&mut tar, &format!("{product_key}/templates/{name}"), &bytes)?;
            }
        }
    }
    let encoder = tar.into_inner()?;
    encoder.finish()?;
    Ok(tar_gz)
}

fn append_tar_bytes<W: std::io::Write>(
    tar: &mut tar::Builder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_cksum();
    tar.append_data(&mut header, path, bytes)
        .with_context(|| format!("adding {path} to Helm chart archive"))
}

/// Pushes an OCI Image Layout directory (or `.oci.tar` tarball) to an OCI
/// Distribution registry (`registry` / `repository`:`tag`), exchanging GHCR
/// `401 WWW-Authenticate: Bearer` challenges when needed, and returns the
/// pushed top-level manifest digest (`sha256:...`).
pub(super) fn push_layout(
    layout: &Path,
    registry: &str,
    repository: &str,
    tag: &str,
    token: Option<&str>,
) -> Result<String> {
    let _extracted;
    let effective_dir = if layout.is_file() {
        let dir = tempfile::tempdir().context("creating OCI archive extraction directory")?;
        let file = std::fs::File::open(layout)
            .with_context(|| format!("opening OCI archive {}", layout.display()))?;
        let mut archive = tar::Archive::new(file);
        archive
            .unpack(dir.path())
            .with_context(|| format!("extracting OCI archive {}", layout.display()))?;
        let path = dir.path().to_path_buf();
        _extracted = Some(dir);
        path
    } else {
        _extracted = None;
        layout.to_path_buf()
    };

    let base = registry.trim_end_matches('/');
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut auth = OciPushAuth {
        agent: &agent,
        repository,
        github_token: token,
        bearer: None,
    };

    let blobs_dir = effective_dir.join("blobs").join("sha256");
    let index_bytes = std::fs::read(effective_dir.join("index.json"))
        .with_context(|| format!("reading {}/index.json", effective_dir.display()))?;
    let index: OciIndex = serde_json::from_slice(&index_bytes).context("parsing index.json")?;

    // Single-entry OCI layout (Helm chart manifest, or `docker buildx --output type=oci`
    // pointing to a nested multi-platform image index).
    if index.manifests.len() == 1 && index.manifests[0].platform.is_none() {
        let entry = &index.manifests[0];
        let entry_bytes = read_blob_file(&blobs_dir, &entry.digest)?;
        if entry.media_type == INDEX_MEDIA_TYPE
            || entry.media_type == "application/vnd.docker.distribution.manifest.list.v2+json"
        {
            let nested: OciIndex = serde_json::from_slice(&entry_bytes)
                .with_context(|| format!("parsing nested OCI index {}", entry.digest))?;
            push_index_children(&mut auth, base, &blobs_dir, &nested)?;
            put_manifest(&mut auth, base, tag, &entry.media_type, &entry_bytes)?;
            return Ok(format!("sha256:{:x}", Sha256::digest(&entry_bytes)));
        }
        let manifest: OciManifest = serde_json::from_slice(&entry_bytes)
            .with_context(|| format!("parsing artifact manifest {}", entry.digest))?;
        push_manifest_blobs(&mut auth, base, &blobs_dir, &manifest)?;
        put_manifest(&mut auth, base, tag, &entry.media_type, &entry_bytes)?;
        return Ok(format!("sha256:{:x}", Sha256::digest(&entry_bytes)));
    }

    push_index_children(&mut auth, base, &blobs_dir, &index)?;
    put_manifest(&mut auth, base, tag, &index.media_type, &index_bytes)?;
    Ok(format!("sha256:{:x}", Sha256::digest(&index_bytes)))
}

struct OciPushAuth<'a> {
    agent: &'a Agent,
    repository: &'a str,
    github_token: Option<&'a str>,
    bearer: Option<String>,
}

impl OciPushAuth<'_> {
    fn current_bearer(&self) -> Option<&str> {
        self.bearer.as_deref().or(self.github_token)
    }

    fn refresh_from_challenge(
        &mut self,
        response: &ureq::http::Response<ureq::Body>,
    ) -> Result<bool> {
        if response.status() != 401 {
            return Ok(false);
        }
        let Some(header) = response
            .headers()
            .get("www-authenticate")
            .and_then(|value| value.to_str().ok())
        else {
            return Ok(false);
        };
        let default_scope = format!("repository:{}:pull,push", self.repository);
        let exchanged = crate::release::publish::oci::exchange_bearer_token(
            self.agent,
            header,
            self.repository,
            &default_scope,
            self.github_token,
        )?;
        self.bearer = Some(exchanged);
        Ok(true)
    }
}

fn read_blob_file(blobs_dir: &Path, digest: &str) -> Result<Vec<u8>> {
    let hex = digest
        .strip_prefix("sha256:")
        .with_context(|| format!("unsupported OCI digest {digest}"))?;
    let path = blobs_dir.join(hex);
    std::fs::read(&path).with_context(|| format!("reading OCI blob {}", path.display()))
}

fn push_index_children(
    auth: &mut OciPushAuth<'_>,
    base: &str,
    blobs_dir: &Path,
    index: &OciIndex,
) -> Result<()> {
    for entry in &index.manifests {
        let manifest_bytes = read_blob_file(blobs_dir, &entry.digest)?;
        let manifest: OciManifest = serde_json::from_slice(&manifest_bytes)
            .with_context(|| format!("parsing child manifest {}", entry.digest))?;
        push_manifest_blobs(auth, base, blobs_dir, &manifest)?;
        put_manifest(
            auth,
            base,
            &entry.digest,
            &entry.media_type,
            &manifest_bytes,
        )?;
    }
    Ok(())
}

fn push_manifest_blobs(
    auth: &mut OciPushAuth<'_>,
    base: &str,
    blobs_dir: &Path,
    manifest: &OciManifest,
) -> Result<()> {
    upload_blob(
        auth,
        base,
        &manifest.config.digest,
        &read_blob_file(blobs_dir, &manifest.config.digest)?,
    )?;
    for layer in &manifest.layers {
        upload_blob(
            auth,
            base,
            &layer.digest,
            &read_blob_file(blobs_dir, &layer.digest)?,
        )?;
    }
    Ok(())
}

fn upload_blob(auth: &mut OciPushAuth<'_>, base: &str, digest: &str, bytes: &[u8]) -> Result<()> {
    let head_url = format!("{base}/v2/{}/blobs/{digest}", auth.repository);
    let mut head = with_auth(auth.agent.head(&head_url), auth.current_bearer()).call()?;
    if auth.refresh_from_challenge(&head)? {
        head = with_auth(auth.agent.head(&head_url), auth.current_bearer()).call()?;
    }
    if head.status() == 200 {
        return Ok(());
    }

    let start_url = format!("{base}/v2/{}/blobs/uploads/", auth.repository);
    let mut started = with_auth(auth.agent.post(&start_url), auth.current_bearer()).send_empty()?;
    if auth.refresh_from_challenge(&started)? {
        started = with_auth(auth.agent.post(&start_url), auth.current_bearer()).send_empty()?;
    }
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
    let mut put = with_auth(auth.agent.put(&put_url), auth.current_bearer())
        .header("Content-Type", "application/octet-stream")
        .send(bytes)?;
    if auth.refresh_from_challenge(&put)? {
        put = with_auth(auth.agent.put(&put_url), auth.current_bearer())
            .header("Content-Type", "application/octet-stream")
            .send(bytes)?;
    }
    let put_status = put.status().as_u16();
    if put_status != 201 && put_status != 202 {
        bail!("{put_url} answered {put_status} uploading {digest}");
    }
    Ok(())
}

fn put_manifest(
    auth: &mut OciPushAuth<'_>,
    base: &str,
    reference: &str,
    media_type: &str,
    bytes: &[u8],
) -> Result<()> {
    let url = format!("{base}/v2/{}/manifests/{reference}", auth.repository);
    let mut response = with_auth(auth.agent.put(&url), auth.current_bearer())
        .header("Content-Type", media_type)
        .send(bytes)?;
    if auth.refresh_from_challenge(&response)? {
        response = with_auth(auth.agent.put(&url), auth.current_bearer())
            .header("Content-Type", media_type)
            .send(bytes)?;
    }
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
