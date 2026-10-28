//! An attempt's staging (ADRs 0003, 0093 and 0103): the outputs a crawl
//! attempt makes are staged with it and commit together with its outcome,
//! or are discarded with it, so a failed attempt leaves none of them. The
//! crawl owns its attempts and their staging; this is the engine's side of
//! it, the interface through which the engine's own operations, such as a
//! Key-value store write a handler makes through its `ctx.run`, stage with
//! an attempt of a Run whose crawl registered its staging on it
//! (`Run::stage_attempt_writes_through`).

use goodfetch_protocol::catalog::ErrorCodeDefinition;
use serde_json::{Map, Value};

/// A Key-value store write staged with an attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StagedWrite {
    /// Sets the key to the JSON value.
    Set {
        /// The key.
        key: String,
        /// Its value, never a top-level null.
        value: Value,
    },
    /// Sets the key to bytes of a content type, kept in its row or, past
    /// what a row holds, as a file of the Run's folder written as the
    /// attempt commits.
    SetBytes {
        /// The key.
        key: String,
        /// Their content type.
        content_type: String,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// Removes the key.
    Delete {
        /// The key.
        key: String,
    },
}

impl StagedWrite {
    /// The key it writes.
    pub fn key(&self) -> &str {
        match self {
            Self::Set { key, .. } | Self::SetBytes { key, .. } | Self::Delete { key } => key,
        }
    }

    /// What it leaves the key holding, as a read through its attempt sees
    /// it.
    pub fn staged(&self) -> StagedKey {
        match self {
            Self::Set { value, .. } => StagedKey::Set(value.clone()),
            Self::SetBytes {
                content_type,
                bytes,
                ..
            } => StagedKey::SetBytes {
                content_type: content_type.clone(),
                bytes: bytes.clone(),
            },
            Self::Delete { .. } => StagedKey::Deleted,
        }
    }
}

/// Why a write was not staged: an Error code of the catalog, its message
/// and its details, as the call that tried it fails.
#[derive(Clone)]
pub struct Unstaged {
    /// The Error code, such as `attempt_ended`.
    pub code: &'static ErrorCodeDefinition,
    /// What happened.
    pub message: String,
    /// The code's details.
    pub details: Map<String, Value>,
}

impl std::fmt::Debug for Unstaged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Unstaged")
            .field("code", &self.code.name)
            .field("message", &self.message)
            .field("details", &self.details)
            .finish()
    }
}

/// What an attempt staged for one key last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StagedKey {
    /// The attempt staged no write of it.
    Untouched,
    /// The attempt staged it set to this value.
    Set(Value),
    /// The attempt staged it set to these bytes, of this content type.
    SetBytes {
        /// Their content type.
        content_type: String,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// The attempt staged its removal.
    Deleted,
}

/// Where the attempts of a Run stage the Key-value store writes their
/// handlers make through `ctx.run`; the attempt is named by its crawl
/// request's ID and its lease epoch.
pub trait AttemptWrites: Send + Sync {
    /// Stages `write` with the attempt, and answers what the attempt had
    /// staged for its key before it.
    ///
    /// # Errors
    ///
    /// Why it was not staged: the attempt ended, or a declared Effect's
    /// body runs in it.
    fn stage(
        &self,
        crawl_request_id: &str,
        lease_epoch: u64,
        write: StagedWrite,
    ) -> Result<StagedKey, Unstaged>;

    /// What the attempt staged for `key` last.
    ///
    /// # Errors
    ///
    /// The attempt ended.
    fn staged(
        &self,
        crawl_request_id: &str,
        lease_epoch: u64,
        key: &str,
    ) -> Result<StagedKey, Unstaged>;
}
