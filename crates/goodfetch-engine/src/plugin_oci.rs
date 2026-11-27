//! A Plugin's OCI artifact (ADR 0033, issue 889): the reference `plugin
//! add` names, pinned by the digest of a multi-platform image index, and
//! the two documents the engine reads on the way to the Plugin's bundle:
//! the index, which names one image manifest per platform, and the host's
//! platform's manifest, whose one layer is the bundle, a tar archive.
//!
//! Each document is a decoder of input Goodfetch did not write (ADR 0067):
//! it is read whole, at most [`DOCUMENT_LIMIT`] bytes, into the few fields
//! the engine follows, every digest parsed, and anything else of it left
//! unread. Fetching the documents and checking their digests is
//! `plugin_install`'s; this module reads bytes and reaches nothing.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::pinned_download::{Digest, MalformedDigest};

/// The media type of an OCI image index.
pub const INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";

/// The media type of an OCI image manifest.
pub const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// The media type of a layer that is a tar archive.
pub const TAR_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar";

/// The media type of a layer that is a gzip-compressed tar archive.
pub const TAR_GZIP_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

/// The OCI artifact and layer media type of a Sigstore provenance attestation
/// bundle attached through the OCI 1.1 Referrers API (ADR 0033, issue 916).
pub const SIGSTORE_BUNDLE_MEDIA_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";

/// The most bytes an index or a manifest may hold: the 4 MiB the OCI
/// distribution specification lets a registry refuse a manifest above.
pub const DOCUMENT_LIMIT: u64 = 4 * 1024 * 1024;

/// The most bytes a Plugin's provenance attestation bundle may hold (1 MiB).
pub const ATTESTATION_LIMIT: u64 = 1024 * 1024;

/// The most bytes a Plugin's bundle layer may hold, compressed or not.
pub const LAYER_LIMIT: u64 = 512 * 1024 * 1024;

/// What the repository's last segment starts with: every Plugin's image is
/// named `goodfetch-plugin-*` (ADR 0033).
pub const REPOSITORY_PREFIX: &str = "goodfetch-plugin-";

/// A Plugin's artifact as `plugin add` names it,
/// `<registry>/<repository>@sha256:<hex>`: the registry's host and port, the
/// repository, whose last segment is `goodfetch-plugin-*`, and the digest of
/// the multi-platform image index it is pinned to. No tag names one: a tag
/// moves, and a project runs the Plugins it was tested with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    registry: String,
    repository: String,
    index: Digest,
}

/// Text that is no Plugin reference, and why.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{reference}` is no Plugin reference `<registry>/goodfetch-plugin-<name>@sha256:<hex>`: {why}"
)]
pub struct MalformedReference {
    /// The text, as given.
    pub reference: String,
    /// What makes it none.
    pub why: &'static str,
}

impl Reference {
    /// The registry's host, and its port when it names one.
    #[must_use]
    pub fn registry(&self) -> &str {
        &self.registry
    }

    /// The repository in the registry.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// The digest of the image index it is pinned to.
    #[must_use]
    pub fn index(&self) -> Digest {
        self.index
    }

    /// The Plugin's name: the repository's last segment after
    /// `goodfetch-plugin-`, which its manifest must give.
    #[must_use]
    pub fn plugin(&self) -> &str {
        self.repository
            .rsplit('/')
            .next()
            .and_then(|last| last.strip_prefix(REPOSITORY_PREFIX))
            .expect("invariant: a Reference's repository ends in `goodfetch-plugin-<name>`")
    }

    /// The path, on the registry, of the manifest or index of digest
    /// `digest` in the repository.
    #[must_use]
    pub fn manifest_path(&self, digest: Digest) -> String {
        format!("/v2/{}/manifests/{digest}", self.repository)
    }

    /// The path, on the registry, of the blob of digest `digest` in the
    /// repository.
    #[must_use]
    pub fn blob_path(&self, digest: Digest) -> String {
        format!("/v2/{}/blobs/{digest}", self.repository)
    }

    /// The OCI 1.1 Referrers API path for the image index this reference
    /// pins, used to discover provenance attestations (ADR 0033, issue 916).
    #[must_use]
    pub fn referrers_path(&self) -> String {
        format!("/v2/{}/referrers/{}", self.repository, self.index)
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}@{}",
            self.registry, self.repository, self.index
        )
    }
}

impl FromStr for Reference {
    type Err = MalformedReference;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let malformed = |why| MalformedReference {
            reference: text.to_owned(),
            why,
        };
        let (name, digest) = text
            .rsplit_once('@')
            .ok_or_else(|| malformed("it names no digest after `@`"))?;
        let index = digest.parse().map_err(|_: MalformedDigest| {
            malformed("its digest is no `sha256:<64 lowercase hex digits>`")
        })?;
        let (registry, repository) = name
            .split_once('/')
            .ok_or_else(|| malformed("it names no registry before the repository"))?;
        if !is_registry(registry) {
            return Err(malformed("its registry is no `host` or `host:port`"));
        }
        if repository.len() > 255 || !repository.split('/').all(is_path_component) {
            return Err(malformed(
                "its repository is no `/`-separated lowercase path of the OCI distribution specification",
            ));
        }
        if !repository
            .rsplit('/')
            .next()
            .is_some_and(|last| last.starts_with(REPOSITORY_PREFIX))
        {
            return Err(malformed(
                "its repository's last segment is no `goodfetch-plugin-*`",
            ));
        }
        Ok(Self {
            registry: registry.to_owned(),
            repository: repository.to_owned(),
            index,
        })
    }
}

/// Whether `registry` is `host` or `host:port`: a host of letters, digits,
/// dots and hyphens, and a port of 1 to 5 digits.
fn is_registry(registry: &str) -> bool {
    let (host, port) = registry
        .split_once(':')
        .map_or((registry, None), |(host, port)| (host, Some(port)));
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-']);
    let port_ok = port.is_none_or(|port| port.parse::<u16>().is_ok_and(|port| port != 0));
    host_ok && port_ok
}

/// Whether `component` is one path component of a repository name:
/// `[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*`.
fn is_path_component(component: &str) -> bool {
    let bytes = component.as_bytes();
    let alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let mut index = 0;
    loop {
        let start = index;
        while index < bytes.len() && alphanumeric(bytes[index]) {
            index += 1;
        }
        if index == start {
            return false;
        }
        if index == bytes.len() {
            return true;
        }
        let separator = index;
        while index < bytes.len() && !alphanumeric(bytes[index]) {
            index += 1;
        }
        let ok = match &bytes[separator..index] {
            b"." | b"_" | b"__" => true,
            hyphens => hyphens.iter().all(|byte| *byte == b'-'),
        };
        if !ok {
            return false;
        }
    }
}

/// An operating system and a CPU architecture, as OCI names them: Go's
/// `GOOS` and `GOARCH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    /// The operating system: `linux`, `darwin`, `windows`.
    pub os: &'static str,
    /// The architecture: `amd64`, `arm64`.
    pub architecture: &'static str,
}

impl Platform {
    /// The platform the engine runs on.
    #[must_use]
    pub fn host() -> Self {
        Self {
            os: match std::env::consts::OS {
                "macos" => "darwin",
                other => other,
            },
            architecture: match std::env::consts::ARCH {
                "x86_64" => "amd64",
                "aarch64" => "arm64",
                "x86" => "386",
                other => other,
            },
        }
    }

    /// Whether a manifest for `os`, `architecture` and `variant` runs
    /// here: the same OS and architecture, and no variant but `v8` on
    /// `arm64`, the only one every `arm64` CPU runs.
    fn runs(self, platform: &RawPlatform) -> bool {
        platform.os == self.os
            && platform.architecture == self.architecture
            && match platform.variant.as_deref() {
                None => true,
                Some(variant) => self.architecture == "arm64" && variant == "v8",
            }
    }
}

/// A document of the artifact the engine refuses to follow.
#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    /// It holds more bytes than [`DOCUMENT_LIMIT`].
    #[error("the {document} holds {size} bytes, above the {DOCUMENT_LIMIT} a Plugin's may hold")]
    TooLarge {
        /// Which document.
        document: &'static str,
        /// How many bytes it holds, or its descriptor says it holds.
        size: u64,
    },
    /// It is no JSON document of its shape.
    #[error("the {document} does not read: {source}")]
    Malformed {
        /// Which document.
        document: &'static str,
        /// Why.
        source: serde_json::Error,
    },
    /// It is of another schema version than 2.
    #[error("the {document} is of schema version {found}, where OCI's is 2")]
    SchemaVersion {
        /// Which document.
        document: &'static str,
        /// The version it declares.
        found: u64,
    },
    /// It, or a descriptor in it, declares another media type than the
    /// engine follows there.
    #[error("the {document} declares the media type `{found}`, where `{expected}` is followed")]
    MediaType {
        /// Which document.
        document: &'static str,
        /// The media type found.
        found: String,
        /// The media type the engine follows there.
        expected: &'static str,
    },
    /// A descriptor names a digest the engine does not check.
    #[error("the {document} names a digest the engine does not check: {source}")]
    Digest {
        /// Which document.
        document: &'static str,
        /// Why.
        source: MalformedDigest,
    },
    /// The index names no manifest for the host's platform.
    #[error("the Plugin's image index names no manifest for {}/{}", platform.os, platform.architecture)]
    NoPlatform {
        /// The host's platform.
        platform: Platform,
    },
    /// The manifest holds other than one layer: a Plugin's bundle is one.
    #[error("the Plugin's image manifest holds {0} layers, where a Plugin's bundle is one")]
    Layers(usize),
    /// The layer holds more bytes than [`LAYER_LIMIT`].
    #[error(
        "the Plugin's bundle layer holds {0} bytes, above the {LAYER_LIMIT} a Plugin's may hold"
    )]
    LayerTooLarge(u64),
}

/// A descriptor of the content the engine fetches next: its digest and
/// size, both checked when it arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    /// The digest of its bytes.
    pub digest: Digest,
    /// How many bytes it holds.
    pub size: u64,
}

/// How the bundle layer's tar archive is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// A plain tar archive.
    None,
    /// A gzip-compressed tar archive.
    Gzip,
}

/// The bundle layer of the host's manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    /// Its digest and size.
    pub descriptor: Descriptor,
    /// How its tar archive is stored.
    pub compression: Compression,
}

/// The fields of a descriptor the engine reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDescriptor {
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default)]
    artifact_type: Option<String>,
    #[serde(default)]
    platform: Option<RawPlatform>,
}

/// The fields of a platform the engine reads.
#[derive(Deserialize)]
struct RawPlatform {
    architecture: String,
    os: String,
    #[serde(default)]
    variant: Option<String>,
}

/// The fields of an image index the engine reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIndex {
    schema_version: u64,
    #[serde(default)]
    media_type: Option<String>,
    manifests: Vec<RawDescriptor>,
}

/// The fields of an image manifest the engine reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawManifest {
    schema_version: u64,
    #[serde(default)]
    media_type: Option<String>,
    layers: Vec<RawDescriptor>,
}

/// How a [`DocumentError`] names the image index.
pub const INDEX: &str = "Plugin's image index";
const MANIFEST: &str = "Plugin's image manifest";

/// Reads `json` as the document `document` of shape `T`, at most
/// [`DOCUMENT_LIMIT`] bytes.
fn read<'a, T: Deserialize<'a>>(
    document: &'static str,
    json: &'a [u8],
) -> Result<T, DocumentError> {
    let size = u64::try_from(json.len()).unwrap_or(u64::MAX);
    if size > DOCUMENT_LIMIT {
        return Err(DocumentError::TooLarge { document, size });
    }
    serde_json::from_slice(json).map_err(|source| DocumentError::Malformed { document, source })
}

/// Holds `found`, a document's version and media type, to OCI's
/// version 2 and `expected`, which a document may leave unsaid.
fn check_header(
    document: &'static str,
    version: u64,
    media_type: Option<String>,
    expected: &'static str,
) -> Result<(), DocumentError> {
    if version != 2 {
        return Err(DocumentError::SchemaVersion {
            document,
            found: version,
        });
    }
    match media_type {
        Some(found) if found != expected => Err(DocumentError::MediaType {
            document,
            found,
            expected,
        }),
        _ => Ok(()),
    }
}

/// Parses the digest of `raw`, a descriptor in `document`.
fn descriptor(document: &'static str, raw: &RawDescriptor) -> Result<Descriptor, DocumentError> {
    Ok(Descriptor {
        digest: raw
            .digest
            .parse()
            .map_err(|source| DocumentError::Digest { document, source })?,
        size: raw.size,
    })
}

/// The descriptor of the manifest for `platform` in the image index
/// `json`: the first that runs there, of the manifest media type, at most
/// [`DOCUMENT_LIMIT`] bytes.
///
/// # Errors
///
/// [`DocumentError`] when `json` is no image index the engine follows or
/// names no manifest for `platform`.
pub fn manifest_for(json: &[u8], platform: Platform) -> Result<Descriptor, DocumentError> {
    let index: RawIndex = read(INDEX, json)?;
    check_header(
        INDEX,
        index.schema_version,
        index.media_type,
        INDEX_MEDIA_TYPE,
    )?;
    let raw = index
        .manifests
        .iter()
        .find(|manifest| {
            manifest
                .platform
                .as_ref()
                .is_some_and(|candidate| platform.runs(candidate))
        })
        .ok_or(DocumentError::NoPlatform { platform })?;
    if raw.media_type != MANIFEST_MEDIA_TYPE {
        return Err(DocumentError::MediaType {
            document: INDEX,
            found: raw.media_type.clone(),
            expected: MANIFEST_MEDIA_TYPE,
        });
    }
    if raw.size > DOCUMENT_LIMIT {
        return Err(DocumentError::TooLarge {
            document: MANIFEST,
            size: raw.size,
        });
    }
    descriptor(INDEX, raw)
}

/// The bundle layer of the image manifest `json`: its one layer, a tar
/// archive, compressed with gzip or not, at most [`LAYER_LIMIT`] bytes.
///
/// # Errors
///
/// [`DocumentError`] when `json` is no image manifest the engine follows.
pub fn bundle_layer(json: &[u8]) -> Result<Layer, DocumentError> {
    let manifest: RawManifest = read(MANIFEST, json)?;
    check_header(
        MANIFEST,
        manifest.schema_version,
        manifest.media_type,
        MANIFEST_MEDIA_TYPE,
    )?;
    let [raw] = manifest.layers.as_slice() else {
        return Err(DocumentError::Layers(manifest.layers.len()));
    };
    let compression = match raw.media_type.as_str() {
        TAR_MEDIA_TYPE => Compression::None,
        TAR_GZIP_MEDIA_TYPE => Compression::Gzip,
        _ => {
            return Err(DocumentError::MediaType {
                document: MANIFEST,
                found: raw.media_type.clone(),
                expected: TAR_GZIP_MEDIA_TYPE,
            });
        }
    };
    if raw.size > LAYER_LIMIT {
        return Err(DocumentError::LayerTooLarge(raw.size));
    }
    Ok(Layer {
        descriptor: descriptor(MANIFEST, raw)?,
        compression,
    })
}

/// Descriptors of provenance attestation manifests (`artifactType` equal to
/// [`SIGSTORE_BUNDLE_MEDIA_TYPE`]) listed in an OCI 1.1 Referrers image index
/// (ADR 0033, issue 916).
///
/// # Errors
///
/// [`DocumentError`] when `json` is no valid OCI image index, or an
/// attestation descriptor exceeds [`DOCUMENT_LIMIT`] or has a malformed
/// digest.
pub fn attestation_manifests(json: &[u8]) -> Result<Vec<Descriptor>, DocumentError> {
    let index: RawIndex = read(INDEX, json)?;
    check_header(
        INDEX,
        index.schema_version,
        index.media_type,
        INDEX_MEDIA_TYPE,
    )?;
    let mut found = Vec::new();
    for raw in &index.manifests {
        if raw.artifact_type.as_deref() != Some(SIGSTORE_BUNDLE_MEDIA_TYPE) {
            continue;
        }
        if raw.media_type != MANIFEST_MEDIA_TYPE {
            return Err(DocumentError::MediaType {
                document: INDEX,
                found: raw.media_type.clone(),
                expected: MANIFEST_MEDIA_TYPE,
            });
        }
        if raw.size > DOCUMENT_LIMIT {
            return Err(DocumentError::TooLarge {
                document: MANIFEST,
                size: raw.size,
            });
        }
        found.push(descriptor(INDEX, raw)?);
    }
    Ok(found)
}

/// The single provenance attestation layer of an attestation image manifest
/// `json`, of media type [`SIGSTORE_BUNDLE_MEDIA_TYPE`] and at most
/// [`ATTESTATION_LIMIT`] bytes (ADR 0033, issue 916).
///
/// # Errors
///
/// [`DocumentError`] when `json` is no attestation manifest the engine
/// follows.
pub fn attestation_layer(json: &[u8]) -> Result<Descriptor, DocumentError> {
    let manifest: RawManifest = read(MANIFEST, json)?;
    check_header(
        MANIFEST,
        manifest.schema_version,
        manifest.media_type,
        MANIFEST_MEDIA_TYPE,
    )?;
    let [raw] = manifest.layers.as_slice() else {
        return Err(DocumentError::Layers(manifest.layers.len()));
    };
    if raw.media_type != SIGSTORE_BUNDLE_MEDIA_TYPE {
        return Err(DocumentError::MediaType {
            document: MANIFEST,
            found: raw.media_type.clone(),
            expected: SIGSTORE_BUNDLE_MEDIA_TYPE,
        });
    }
    if raw.size > ATTESTATION_LIMIT {
        return Err(DocumentError::TooLarge {
            document: MANIFEST,
            size: raw.size,
        });
    }
    descriptor(MANIFEST, raw)
}

/// The fuzz target's body for OCI image indexes and manifests (ADR 0067):
/// reads `data` as an index, for every platform the engine ships on, and
/// as a manifest, and parses it as a reference; a reference it reads must
/// read back the same from its text. It may refuse `data`, never panic.
pub fn fuzz_oci_image_index(data: &[u8]) {
    for (os, architecture) in [
        ("linux", "amd64"),
        ("linux", "arm64"),
        ("darwin", "arm64"),
        ("windows", "amd64"),
    ] {
        let _ = manifest_for(data, Platform { os, architecture });
    }
    let _ = bundle_layer(data);
    let _ = attestation_manifests(data);
    let _ = attestation_layer(data);
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(reference) = text.parse::<Reference>()
    {
        assert_eq!(
            reference.to_string().parse::<Reference>().as_ref(),
            Ok(&reference),
            "a reference reads back differently from its text"
        );
    }
}

#[cfg(test)]
mod tests;
