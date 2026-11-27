//! The published Operation catalog file (`goodfetch-catalog.json`), attached to
//! every GitHub Release for both Products (ADRs 0030, 0032 and 0095; issue
//! 1662): a self-contained JSON document holding both Products' catalog entries
//! with their `product` marks, the protocol version, the Connection framing
//! types outside the catalog, and every JSON Schema definition they refer to.
//!
//! Readers call [`PublishedCatalog::version_of`] first and hold it to the
//! `published-catalog` format's rule (ADR 0066), or use
//! [`PublishedCatalog::read`] which checks the version before parsing the rest.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::ProtocolVersion;

/// The file name of the published Operation catalog on every GitHub Release.
pub const PUBLISHED_CATALOG_FILE: &str = "goodfetch-catalog.json";

/// The `published-catalog` format's current version, `major.minor` (ADRs 0030
/// and 0066).
pub const PUBLISHED_CATALOG_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The JSON Schema draft `$defs` conforms to.
pub const SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The self-contained Operation catalog published with every release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishedCatalog {
    /// The version of the `published-catalog` format this file was written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format_version: ProtocolVersion,
    /// The protocol version this release's engines speak.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The JSON Schema draft of `$defs`.
    #[serde(rename = "$schema")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schema: String,
    /// Every catalog section of both Products (`operations`, `events`,
    /// `error_codes`, `exit_codes`, `protocol_failures`, `plugin_kinds`,
    /// `metrics`, `settings`, `hook_stages`, `shared_types`, `kind_parts`, …),
    /// each entry carrying its `product` mark (`collie` or `foxhound`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub catalog: BTreeMap<String, Value>,
    /// The Connection handshake and framing types outside the catalog (`hello`,
    /// `cancel`, `credit`, `events`, `chunk`, `meta`, …).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub connection: BTreeMap<String, Value>,
    /// Every JSON Schema definition (`#/$defs/...`) reached by `catalog` and
    /// `connection`.
    #[serde(rename = "$defs")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub defs: BTreeMap<String, Value>,
}

/// Why reading a published Operation catalog failed.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PublishedCatalogError {
    /// The file is not valid JSON or lacks a valid `format_version` or required
    /// fields.
    #[error("malformed published catalog: {0}")]
    Malformed(#[from] serde_json::Error),
    /// The file's `format_version` is outside what this reader accepts.
    #[error(
        "published catalog format version {found} is unsupported (this reader supports {supported})"
    )]
    UnsupportedVersion {
        /// The `format_version` declared by the file.
        found: ProtocolVersion,
        /// The version this reader supports.
        supported: ProtocolVersion,
    },
}

/// Reads `format_version` alone, ignoring every other field so an unsupported
/// version is refused before any new field fails deserialization.
#[derive(Deserialize)]
struct VersionHeader {
    format_version: ProtocolVersion,
}

impl PublishedCatalog {
    /// Reads only `format_version` from `bytes`, before any other field.
    ///
    /// # Errors
    ///
    /// Returns [`PublishedCatalogError::Malformed`] when `bytes` is not a JSON
    /// object carrying a valid `format_version`.
    pub fn version_of(bytes: &[u8]) -> Result<ProtocolVersion, PublishedCatalogError> {
        let header: VersionHeader = serde_json::from_slice(bytes)?;
        Ok(header.format_version)
    }

    /// Reads and validates a published catalog, refusing an unsupported
    /// `format_version` before parsing the rest of the document.
    ///
    /// # Errors
    ///
    /// Returns [`PublishedCatalogError::UnsupportedVersion`] when
    /// `format_version` is incompatible with [`PUBLISHED_CATALOG_VERSION`]
    /// under the default rule (ADR 0030), or
    /// [`PublishedCatalogError::Malformed`] when the JSON does not match the
    /// schema.
    pub fn read(bytes: &[u8]) -> Result<Self, PublishedCatalogError> {
        let found = Self::version_of(bytes)?;
        if !is_compatible(PUBLISHED_CATALOG_VERSION, found) {
            return Err(PublishedCatalogError::UnsupportedVersion {
                found,
                supported: PUBLISHED_CATALOG_VERSION,
            });
        }
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// Default compatibility rule (ADR 0030): reads its own major up to its own
/// minor; during `0.y` previews each `0.y` counts as its own major.
fn is_compatible(reader: ProtocolVersion, found: ProtocolVersion) -> bool {
    if reader.major == 0 || found.major == 0 {
        reader == found
    } else {
        found.major == reader.major && found.minor <= reader.minor
    }
}
