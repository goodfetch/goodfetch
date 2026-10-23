//! Certificate verification's contracts (ADRs 0016 and 0042): the
//! per-domain exceptions `tls.trusted_cas` and `tls.accept_invalid_certs`,
//! each a map from a host or a registrable domain with its subdomains to a
//! value, and `certificate_invalid`, the Error code a certificate that fails
//! verification ends a request with, naming the host, the failure and the
//! setting that makes an exception. No setting turns verification off.
//!
//! A certificate a domain's trust-on-first-use exception accepted is pinned
//! by the SHA-256 of its `SubjectPublicKeyInfo` in the user's known-certificates
//! store, one partition per person; `certs.list` reads the caller's pins.
//! A pinned host that presents another key, or whose pin row the engine
//! cannot read, stops with `certificate_changed` until a person accepts the
//! new key with `certs.accept`; `certs.forget` removes a pin. Both are a
//! person's answers, offered only in the Console and an interactive CLI
//! prompt (ADRs 0011 and 0042).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, Stability, ValueKind, schema_of,
};
use crate::run::{PointInTime, RunId};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

fn empty_table() -> Value {
    json!({})
}

/// `tls.trusted_cas` (ADR 0042).
pub const TLS_TRUSTED_CAS: Setting = Setting {
    name: "tls.trusted_cas",
    description: "Extra certificate authorities by domain: a host or a registrable domain with its subdomains, mapped to certificate files or Secret references, trusted beside the default roots for that domain and nowhere else. Taken only from an Operator policy and user and project configuration, never from a Recipe or a run; on a Daemon with accounts only the operator role writes it, in the Operator policy.",
    kind: ValueKind::Map(&ValueKind::List(&ValueKind::Text)),
    default: empty_table,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

fn trust_on_first_use_only(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|entries| entries.values().all(Value::is_null))
}

fn accepts_any_invalid_certificate(value: &Value) -> bool {
    value.as_object().is_some_and(|entries| !entries.is_empty())
}

/// `tls.accept_invalid_certs` (ADR 0042).
pub const TLS_ACCEPT_INVALID_CERTS: Setting = Setting {
    name: "tls.accept_invalid_certs",
    description: "Domains whose certificate may fail verification, each with a pin, `any`, or `null` for trust on first use. A pin and `any` are taken only from an Operator policy and user and project configuration; a Recipe or a run may list a domain only for trust on first use.",
    kind: ValueKind::Map(&ValueKind::Optional(&ValueKind::Text)),
    default: empty_table,
    clearable: false,
    authority: Authority::ConfigurationExcept(trust_on_first_use_only),
    weakens: Some(accepts_any_invalid_certificate),
    arrived_in: ARRIVED_IN,
};

/// A certificate failed verification.
pub const CERTIFICATE_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "certificate_invalid",
    description: "A host's certificate failed verification and no exception covers its domain, so nothing was sent to it. The details name the host, the failure (`expired`, `not_yet_valid`, `unknown_issuer`, `name_mismatch`, `self_signed`, `revoked`, or another failure as text) and, in `fix`, the setting that makes an exception for that domain.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// How a certificate failed verification, as the wire names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KnownCertificateFailure {
    /// Its validity has ended.
    Expired,
    /// Its validity has not begun.
    NotYetValid,
    /// It chains to no authority trusted for the host.
    UnknownIssuer,
    /// It does not name the host.
    NameMismatch,
    /// Its own key signed it.
    SelfSigned,
    /// Its issuer revoked it.
    Revoked,
}

impl KnownCertificateFailure {
    /// Its name as the wire writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::NotYetValid => "not_yet_valid",
            Self::UnknownIssuer => "unknown_issuer",
            Self::NameMismatch => "name_mismatch",
            Self::SelfSigned => "self_signed",
            Self::Revoked => "revoked",
        }
    }
}

/// How a certificate failed verification: a failure the wire names, or
/// another one as text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CertificateFailure {
    /// A failure the wire names.
    Known(KnownCertificateFailure),
    /// Any other failure, as the verifier described it.
    Unrecognized(String),
}

impl CertificateFailure {
    /// Its text as the wire writes it.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(known) => known.as_str(),
            Self::Unrecognized(text) => text,
        }
    }
}

/// What `certificate_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CertificateInvalidDetails {
    /// The host whose certificate failed, as the request named it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// How it failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: CertificateFailure,
    /// The setting that makes an exception for the host's domain, and
    /// where it is set.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// One pin of the known-certificates store, as `certs.list` answers it and
/// as the store's rows hold it: a host whose certificate was accepted on
/// first use, and the key it must keep presenting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CertificatePin {
    /// The host, as the URL parser leaves it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// The SHA-256 of the pinned certificate's `SubjectPublicKeyInfo`, as
    /// `sha256/` and its standard base64; none when the row cannot be read,
    /// which stops the host as a changed key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub spki_sha256: Option<String>,
    /// When it was pinned; none when the row cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pinned_at: Option<PointInTime>,
    /// The Run whose connection pinned it; none outside a Run or when the
    /// row cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// The SHA-256 of the key the host presented instead, which stopped it
    /// with `certificate_changed`, as `sha256/` and its standard base64: the
    /// key `certs.accept` pins in its place. None while no change waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub changed_spki_sha256: Option<String>,
}

/// A pinned host stopped.
pub const CERTIFICATE_CHANGED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "certificate_changed",
    description: "A pinned host presented a certificate whose public key is not its pin, or its pin row cannot be read, so nothing was sent to it, nor is for the rest of the run. The details carry the host, both SHA-256 hashes and the fix: a person accepts the new key with `certs accept` in an interactive prompt or the Console, which no agent, Recipe or `--json` command can do.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `certificate_changed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CertificateChangedDetails {
    /// The host, as the URL parser leaves it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// The SHA-256 of the pinned key, as `sha256/` and its standard
    /// base64; none when its pin row cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pinned_spki_sha256: Option<String>,
    /// The SHA-256 of the key the host presented, written the same way.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub presented_spki_sha256: String,
    /// What a person does to let the host through again.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `certs.accept` takes: the host, and the changed key the person
/// accepts, as `certs.list` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CertsAcceptParams {
    /// The pinned host, as the URL parser leaves it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// The SHA-256 of the changed key, as `sha256/` and its standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub spki_sha256: String,
}

/// `certs.accept` (ADR 0042).
pub const CERTS_ACCEPT: Operation = Operation {
    name: "certs.accept",
    description: "Pins the changed key that stopped a host with `certificate_changed` in place of its pin, in the caller's partition, and records the accepted change in the Audit log; answers the new pin. Refused unless the key is the change the store holds for the host. A person's answer: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<CertsAcceptParams>,
    result: schema_of::<CertificatePin>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `certs.forget` takes: the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CertsForgetParams {
    /// The pinned host, as the URL parser leaves it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
}

/// What `certs.forget` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CertsForgotten {
    /// The host.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// Whether the caller had a pin for it, now removed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub forgotten: bool,
}

/// `certs.forget` (ADR 0042).
pub const CERTS_FORGET: Operation = Operation {
    name: "certs.forget",
    description: "Removes the caller's pin of a host, and any change waiting for it, so its next connection under trust on first use pins again; nothing else removes a pin. A person's answer: offered only in the Console and the interactive CLI prompt.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<CertsForgetParams>,
    result: schema_of::<CertsForgotten>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `certs.list` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CertsListParams {}

/// What `certs.list` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CertsListed {
    /// The caller's pins, by host.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pins: Vec<CertificatePin>,
}

/// `certs.list` (ADR 0042).
pub const CERTS_LIST: Operation = Operation {
    name: "certs.list",
    description: "Lists the caller's pins in the known-certificates store: each host whose invalid certificate was accepted on first use, the SHA-256 of its public key, when and by which Run. Another person's pins on a shared Daemon are never listed. A row the engine cannot read is listed without its hash; it stops its host as a changed key.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<CertsListParams>,
    result: schema_of::<CertsListed>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CERTS_LIST, CERTS_ACCEPT, CERTS_FORGET],
    events: &[],
    handler_requests: &[],
    error_codes: &[CERTIFICATE_INVALID, CERTIFICATE_CHANGED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[TLS_TRUSTED_CAS, TLS_ACCEPT_INVALID_CERTS],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
