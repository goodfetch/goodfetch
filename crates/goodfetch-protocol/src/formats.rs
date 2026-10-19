//! The format registry (ADR 0066): every versioned format an engine reads
//! or a peer speaks, with its current `major.minor`, how its Compatibility
//! corpus stores it, its schema and the rule of ADR 0030 it follows, and one
//! compatibility function per rule, which decides each pair of versions.
//! `xtask codegen` writes the registry into the bundle as `formats`, which
//! `xtask compat` reads.

use crate::browser_cache::BrowserCacheEntry;
use crate::catalog::{PROTOCOL_VERSION, ProtocolVersion, SchemaOf, schema_of};
use crate::jsonrpc::Message;

/// How a format's files are stored in its corpus sets (ADR 0066).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// JSON documents.
    Json,
    /// A SQLite-backed store, as SQL text dumps.
    Sqlite,
    /// A protocol's recorded transcripts, one message per line.
    Transcript,
}

impl Storage {
    /// Its name in the bundle.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Sqlite => "sqlite",
            Self::Transcript => "transcript",
        }
    }
}

/// A rule of ADR 0030 by which a reader treats another version of its
/// format. Each later rule (the frozen-major family of the Usage ledger, the
/// Response cache's, the Clean-up rules') joins with the first format that
/// follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A reader reads its own major up to its own minor.
    Default,
}

impl Rule {
    /// Its name in the bundle.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
        }
    }

    /// What a reader at `reader` makes of a file, or a peer's `hello`, at
    /// `found`.
    pub fn compatibility(self, reader: ProtocolVersion, found: ProtocolVersion) -> Compatibility {
        match self {
            Self::Default => {
                if found.major < reader.major {
                    Compatibility::OlderMajor
                } else if found.major > reader.major {
                    Compatibility::NewerMajor
                } else if found.minor > reader.minor {
                    Compatibility::NewerMinor
                } else {
                    Compatibility::Compatible
                }
            }
        }
    }
}

/// A compatibility function's verdict on one pair of versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    /// The reader reads it.
    Compatible,
    /// It is of a newer minor than the reader knows: upgrade the reader.
    NewerMinor,
    /// It is of an older major: upgrade it (`upgrade-files` for a file, a
    /// newer Client for a protocol).
    OlderMajor,
    /// It is of a newer major: upgrade the reader.
    NewerMajor,
}

/// One registered format.
pub struct Format {
    /// Its name, the folder of its corpus under `compat/`.
    pub name: &'static str,
    /// Its current version.
    pub version: ProtocolVersion,
    /// How its corpus stores it.
    pub storage: Storage,
    /// The rule it follows.
    pub rule: Rule,
    /// The type of one document, row or message.
    pub schema: SchemaOf,
}

/// The JSON-RPC protocol every Client speaks to an engine (ADRs 0024 and
/// 0066), at the protocol's version.
pub const PROTOCOL: Format = Format {
    name: "protocol",
    version: PROTOCOL_VERSION,
    storage: Storage::Transcript,
    rule: Rule::Default,
    schema: schema_of::<Message>,
};

/// The Browser cache's entries (ADRs 0066 and 0098), one `entry.json` per
/// Pinned artifact: an engine reads its own major up to its own minor, and
/// never deletes an entry it cannot read.
pub const BROWSER_CACHE: Format = Format {
    name: "browser-cache",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Json,
    rule: Rule::Default,
    schema: schema_of::<BrowserCacheEntry>,
};

/// Every registered format.
pub const FORMATS: &[&Format] = &[&PROTOCOL, &BROWSER_CACHE];

#[cfg(test)]
mod tests;
