//! Parsing Checkpoint locations: local paths vs `s3://`, `gs://`, `az://` URLs.

use std::fmt::Write as _;
use std::path::PathBuf;

/// An object-storage scheme supported for Checkpoints (issue 3472).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// Amazon S3 and S3-compatible stores (`s3://bucket/key`).
    S3,
    /// Google Cloud Storage (`gs://bucket/object`).
    Gs,
    /// Azure Blob Storage (`az://container/blob`).
    Az,
}

impl Scheme {
    /// The scheme prefix without `://`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Gs => "gs",
            Self::Az => "az",
        }
    }
}

/// A validated object-storage location (`<scheme>://<bucket>/<key>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectLocation {
    /// The store scheme.
    pub scheme: Scheme,
    /// Bucket (S3/GCS) or container (Azure).
    pub bucket: String,
    /// Object key or blob name (non-empty, without leading `/`).
    pub key: String,
    /// Original URL string, reported back in `Checkpoint.location`.
    pub raw: String,
}

/// A parsed Checkpoint location: either a local filesystem path or an
/// object-storage URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A path on the engine's machine.
    Local(PathBuf),
    /// An object-storage URL (`s3://`, `gs://`, `az://`).
    Object(ObjectLocation),
}

/// Why an object-storage URL is invalid.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LocationError {
    /// Missing bucket/container name.
    #[error("the {scheme}:// Checkpoint location `{raw}` names no bucket or container")]
    MissingBucket {
        /// Scheme name.
        scheme: &'static str,
        /// Original input.
        raw: String,
    },
    /// Missing object key / blob name.
    #[error("the {scheme}:// Checkpoint location `{raw}` names no object key")]
    MissingKey {
        /// Scheme name.
        scheme: &'static str,
        /// Original input.
        raw: String,
    },
    /// Disallowed query string or fragment in the object URL.
    #[error("the {scheme}:// Checkpoint location `{raw}` must not contain a query or fragment")]
    QueryOrFragment {
        /// Scheme name.
        scheme: &'static str,
        /// Original input.
        raw: String,
    },
}

impl Location {
    /// Parses `raw` as an object-storage URL when it starts with `s3://`,
    /// `gs://`, or `az://`, else as a local filesystem path.
    ///
    /// # Errors
    ///
    /// [`LocationError`] when `raw` starts with a known object-storage scheme
    /// but is missing a bucket/container or key, or carries a query/fragment.
    pub fn parse(raw: &str) -> Result<Self, LocationError> {
        let (scheme, rest) = if let Some(rest) = raw.strip_prefix("s3://") {
            (Scheme::S3, rest)
        } else if let Some(rest) = raw.strip_prefix("gs://") {
            (Scheme::Gs, rest)
        } else if let Some(rest) = raw.strip_prefix("az://") {
            (Scheme::Az, rest)
        } else {
            return Ok(Self::Local(PathBuf::from(raw)));
        };
        if rest.contains('?') || rest.contains('#') {
            return Err(LocationError::QueryOrFragment {
                scheme: scheme.as_str(),
                raw: raw.to_owned(),
            });
        }
        let Some((bucket, key)) = rest.split_once('/') else {
            return Err(if rest.is_empty() {
                LocationError::MissingBucket {
                    scheme: scheme.as_str(),
                    raw: raw.to_owned(),
                }
            } else {
                LocationError::MissingKey {
                    scheme: scheme.as_str(),
                    raw: raw.to_owned(),
                }
            });
        };
        if bucket.is_empty() {
            return Err(LocationError::MissingBucket {
                scheme: scheme.as_str(),
                raw: raw.to_owned(),
            });
        }
        let key = key.trim_start_matches('/');
        if key.is_empty() {
            return Err(LocationError::MissingKey {
                scheme: scheme.as_str(),
                raw: raw.to_owned(),
            });
        }
        Ok(Self::Object(ObjectLocation {
            scheme,
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            raw: raw.to_owned(),
        }))
    }
}

/// Percent-encodes a single URI segment (encoding `/` as `%2F`).
pub(super) fn encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Percent-encodes an object key path, preserving `/` separators.
pub(super) fn encode_path(key: &str) -> String {
    key.split('/')
        .map(encode_component)
        .collect::<Vec<_>>()
        .join("/")
}
