//! Saved identities (ADRs 0015, 0035, 0056, 0089, 0092, 0098 and 0099):
//! encrypted, named login state kept in a per-install, per-major store shared
//! by both Products.
//!
//! `identity.list`, `identity.import`, `identity.delete` and `identity.export`
//! are reached through the Connection's `identities` and the `identity` CLI
//! commands of both executables; `identity.export` writes ADR 0015's
//! `age`-encrypted `session-export` bundle, while `identity.export_storage_state`
//! is person-only (`Reach::PersonOnly`) and writes clear-text Playwright 1.64
//! `storageState` only after an interactive confirmation at a terminal.

use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ARRIVED_IN;
use super::proxy::ProxyKind;
use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    Reach, Setting, SharedType, SharedTypeValue, Stability, ValueKind, schema_of,
};
use crate::point_in_time::PointInTime;
use crate::settings::session_source::SessionSource;

/// Default maximum unused age for a Saved identity: 30 days in milliseconds
/// (ADR 0035).
pub const DEFAULT_MAX_UNUSED_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

fn default_max_unused_age() -> Value {
    Value::from(DEFAULT_MAX_UNUSED_AGE_MS)
}

/// `saved_identity.max_unused_age` (ADR 0035).
pub const SAVED_IDENTITY_MAX_UNUSED_AGE: Setting = Setting {
    name: "saved_identity.max_unused_age",
    description: "How long an unused Saved identity stays in the install's store before it expires and is deleted, in milliseconds: 30 days (2592000000 ms) by default; `null` disables expiry. Each expiry deletion is recorded in the Audit log. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Duration,
    default: default_max_unused_age,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Non-secret metadata about one Saved identity in the per-major store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedIdentityInfo {
    /// The Saved identity's unique name within this Product major's store.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The Access mode it was saved under (`standard`, `stealth`, or `identified`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub access_mode: String,
    /// The browser family (`chromium`, `firefox`, or `webkit`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_family: String,
    /// Its BCP 47 locale (language), when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locale: Option<String>,
    /// Its IANA time zone ID, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timezone_id: Option<String>,
    /// The distinct cookie and storage domains held (never secret values).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domains: Vec<String>,
    /// Number of stored cookies.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cookie_count: u64,
    /// Number of stored origins.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin_count: u64,
    /// Whether the Saved identity was saved from a Session that was lent via
    /// `page.lend()` (ADR 0089).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub was_lent: bool,
    /// Whether the Saved identity is bound to this machine's identity (ADR 0056).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub machine_bound: bool,
    /// Whether the Saved identity is bound to a proxy (ADR 0035).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_bound: bool,
    /// The Session source kind it was imported from, when created by import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source_kind: Option<String>,
    /// When the Saved identity was first created.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub created_at: SystemTime,
    /// When the Saved identity was last written or replaced.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub updated_at: SystemTime,
    /// When the Saved identity was last saved or reused.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub last_used_at: SystemTime,
    /// When the Saved identity will expire if unused; `null` when expiry is disabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::point_in_time::nullable"
    )]
    #[schemars(with = "Option<String>", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub expires_at: Option<SystemTime>,
    /// Non-fatal warnings produced while importing this Saved identity (such as
    /// excluded device-bound session cookies or browser-family differences, ADR 0056).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub warnings: Vec<String>,
}

/// What `identity.list` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityListParams {}

/// What `identity.list` answers: every non-expired Saved identity in this
/// Product major's store, ordered by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IdentityList {
    /// Every non-expired Saved identity, metadata only.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub identities: Vec<SavedIdentityInfo>,
}

/// What `identity.import` takes: the Saved identity's name, its Session source,
/// and optional identity/proxy overrides allowed at import time (ADR 0056).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityImportParams {
    /// The name under which to store the imported Saved identity.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The Session source to import (`state_file`, `cookie_header`, etc.).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: SessionSource,
    /// Optional Access mode at import (`standard` by default, or `stealth`;
    /// `identified` is refused per ADR 0056). Ignored for a `session_export`
    /// bundle, which keeps its saved Access mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub access_mode: Option<String>,
    /// Optional BCP 47 locale (language) of the source browser; inferred from
    /// the host when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub locale: Option<String>,
    /// Optional IANA time zone ID of the source browser; inferred from the
    /// host when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub timezone_id: Option<String>,
    /// Optional proxy server (`scheme://host:port`, without credentials) bound
    /// to this imported identity (ADR 0056).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_server: Option<String>,
    /// Optional ISO 3166-1 alpha-2 country code of the proxy binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_country: Option<String>,
    /// Optional proxy kind of the proxy binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_kind: Option<ProxyKind>,
}

/// What `identity.delete` takes: the Saved identity's name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityDeleteParams {
    /// The Saved identity's name to remove from the store.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// What `identity.delete` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IdentityDeleted {
    /// The Saved identity's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Whether a Saved identity of that name existed and was deleted.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub deleted: bool,
}

/// What `identity.export` takes: the Saved identity's name and exactly one
/// `age` recipient (`public_key` or `passphrase`, a Secret reference, ADR 0015).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityExportParams {
    /// The Saved identity's name to export.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// An `age` public key (`age1…`) to encrypt the session export bundle to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub public_key: Option<String>,
    /// A Secret reference whose value is the passphrase to encrypt the session
    /// export bundle with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub passphrase: Option<String>,
}

/// What `identity.export` answers: the ASCII-armored `age`-encrypted
/// `session-export` bundle and its SHA-256 digest (ADR 0015).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IdentityExported {
    /// The Saved identity's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The ASCII-armored `age`-encrypted `session-export` bundle.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bundle: String,
    /// Lowercase hex SHA-256 digest of `bundle`, matching the Audit log entry.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// Number of cookies carried in the bundle.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cookie_count: u64,
    /// Number of origins carried in the bundle.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin_count: u64,
}

/// What `identity.export_storage_state` takes (person-only, ADR 0056).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityExportStorageStateParams {
    /// The Saved identity's name to export as clear-text Playwright `storageState`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// What `identity.export_storage_state` answers (person-only, ADR 0056).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IdentityStorageStateExported {
    /// The Saved identity's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Clear-text Playwright 1.64 `storageState` JSON document.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub storage_state: String,
    /// Number of cookies carried in the document.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub cookie_count: u64,
    /// Number of origins carried in the document.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin_count: u64,
}

/// What `saved_identity_not_found` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedIdentityNotFoundDetails {
    /// The Saved identity name that was not found or had expired.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// What `saved_identity_mismatch` details (ADR 0035).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SavedIdentityMismatchDetails {
    /// The Saved identity's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Which binding mismatched (`access_mode` or `proxy`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub field: String,
    /// What the Saved identity was saved with.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expected: String,
    /// What the opening Session asked for.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub actual: String,
    /// What fixes the call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// `saved_identity_not_found`.
pub const SAVED_IDENTITY_NOT_FOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "saved_identity_not_found",
    description: "No Saved identity of that name exists in this Product major's store, or it expired after going unused past `saved_identity.max_unused_age` (ADR 0035).",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `saved_identity_mismatch`.
pub const SAVED_IDENTITY_MISMATCH: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "saved_identity_mismatch",
    description: "The Saved identity cannot be reused under a different Access mode or an inconsistent proxy binding (ADR 0035); details name what it was saved with.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `identity.list`.
pub const IDENTITY_LIST: Operation = Operation {
    name: "identity.list",
    description: "Lists every non-expired Saved identity in this Product major's store, with its metadata and domains, never secret values (ADR 0035).",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<IdentityListParams>,
    result: schema_of::<IdentityList>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `identity.import`.
pub const IDENTITY_IMPORT: Operation = Operation {
    name: "identity.import",
    description: "Imports cookies and storage from a Session source (`state_file`, `cookie_header`, etc.) into the encrypted per-major Saved identity store under `name`, replacing any existing identity of that name under the install-wide lock and recording the import in the Audit log (ADRs 0035 and 0056).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<IdentityImportParams>,
    result: schema_of::<SavedIdentityInfo>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `identity.delete`.
pub const IDENTITY_DELETE: Operation = Operation {
    name: "identity.delete",
    description: "Deletes the named Saved identity from this Product major's encrypted store, wiping its secret fields and recording the deletion in the Audit log (ADR 0035).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<IdentityDeleteParams>,
    result: schema_of::<IdentityDeleted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `identity.export`.
pub const IDENTITY_EXPORT: Operation = Operation {
    name: "identity.export",
    description: "Exports the named Saved identity as an `age`-encrypted `session-export` bundle to an `age` public key or a passphrase Secret reference, preserving its Access mode, Identity profile seed, proxy binding and machine binding, and recording the export in the Audit log (ADRs 0015, 0035 and 0056).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<IdentityExportParams>,
    result: schema_of::<IdentityExported>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `identity.export_storage_state` (person-only, ADR 0056).
pub const IDENTITY_EXPORT_STORAGE_STATE: Operation = Operation {
    name: "identity.export_storage_state",
    description: "Exports the named Saved identity as clear-text Playwright 1.64 `storageState` JSON; person-only at an interactive terminal, never offered on SDK or agent surfaces, and recorded in the Audit log (ADR 0056).",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<IdentityExportStorageStateParams>,
    result: schema_of::<IdentityStorageStateExported>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

const fn audit_kind(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::AuditKind,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

/// A Saved identity was saved or imported.
pub const SAVED_IDENTITY_SAVED: SharedTypeValue = audit_kind(
    "saved_identity_saved",
    "A Session's login or imported state was encrypted and stored as a named Saved identity (ADR 0035).",
);

/// A Saved identity was reused to open a Session.
pub const SAVED_IDENTITY_REUSED: SharedTypeValue = audit_kind(
    "saved_identity_reused",
    "A Saved identity was opened and reused by a Session (ADR 0035).",
);

/// A Saved identity was deleted or expired.
pub const SAVED_IDENTITY_DELETED: SharedTypeValue = audit_kind(
    "saved_identity_deleted",
    "A Saved identity was deleted from the store, either explicitly or on expiry after going unused (ADR 0035).",
);

/// A Saved identity was exported.
pub const SAVED_IDENTITY_EXPORTED: SharedTypeValue = audit_kind(
    "saved_identity_exported",
    "A Saved identity was exported as an encrypted session bundle or, by a person at a terminal, as clear-text `storageState` (ADRs 0015 and 0056).",
);

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        IDENTITY_LIST,
        IDENTITY_IMPORT,
        IDENTITY_DELETE,
        IDENTITY_EXPORT,
        IDENTITY_EXPORT_STORAGE_STATE,
    ],
    events: &[],
    handler_requests: &[],
    error_codes: &[SAVED_IDENTITY_NOT_FOUND, SAVED_IDENTITY_MISMATCH],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[SAVED_IDENTITY_MAX_UNUSED_AGE],
    hook_stages: &[],
    shared_type_values: &[
        SAVED_IDENTITY_SAVED,
        SAVED_IDENTITY_REUSED,
        SAVED_IDENTITY_DELETED,
        SAVED_IDENTITY_EXPORTED,
    ],
};
