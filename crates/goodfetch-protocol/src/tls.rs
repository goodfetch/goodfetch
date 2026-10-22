//! Certificate verification's contracts (ADRs 0016 and 0042): the
//! per-domain exceptions `tls.trusted_cas` and `tls.accept_invalid_certs`,
//! each a map from a host or a registrable domain with its subdomains to a
//! value, and `certificate_invalid`, the Error code a certificate that fails
//! verification ends a request with, naming the host, the failure and the
//! setting that makes an exception. No setting turns verification off.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};

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

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[CERTIFICATE_INVALID],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[TLS_TRUSTED_CAS, TLS_ACCEPT_INVALID_CERTS],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
