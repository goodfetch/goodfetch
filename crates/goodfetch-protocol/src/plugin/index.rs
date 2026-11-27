//! The Plugin index (ADRs 0033, 0047, 0052 and 0066; issue 911): a signed,
//! versioned, expiring JSON document carrying listed Plugins, their signer
//! identities and conformance results, and the revocation list of digests
//! and signers.
//!
//! The index has no `major.minor` format version (ADR 0066): freshness is
//! carried by its monotonic `version` integer and its 14-day `expires_at`
//! timestamp, verified under a signature bundle tied to the repository's
//! workflow identity.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, SharedType, SharedTypeValue, Stability, ValueKind, ValueShape,
    schema_of,
};
use crate::plugin::PluginKindName;
use crate::plugin::approvals::{Origin, is_false};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The fixed default URL where the official signed Plugin index is
/// published (ADR 0033).
pub const DEFAULT_PLUGIN_INDEX_URL: &str = "https://plugins.goodfetch.dev/plugin-index/index.json";

/// How many days a signed Plugin index remains valid after signing
/// (ADR 0033).
pub const INDEX_LIFETIME_DAYS: u64 = 14;

/// What `plugin.index.url` accepts.
const INDEX_URL_FORM: &str = "an `https://` URL with a host and no query, fragment or credentials";

/// Validates an HTTPS URL with a host and no query, fragment, whitespace or
/// embedded credentials.
fn check_index_url(value: &Value) -> Result<(), String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("{INDEX_URL_FORM}, found `{value}`"))?;
    let host_and_path = text
        .strip_prefix("https://")
        .ok_or_else(|| format!("{INDEX_URL_FORM}, found `{text}`"))?;
    let host = host_and_path.split('/').next().unwrap_or_default();
    let bad = host.is_empty()
        || host.contains('@')
        || text.contains('?')
        || text.contains('#')
        || text.chars().any(char::is_whitespace);
    if bad {
        return Err(format!("{INDEX_URL_FORM}, found `{text}`"));
    }
    Ok(())
}

/// `plugin.index.url`'s value shape.
pub const PLUGIN_INDEX_URL_SHAPE: ValueShape = ValueShape {
    name: "PluginIndexUrl",
    check: check_index_url,
    schema: || {
        json!({
            "type": "string",
            "pattern": "^https://[^/?#@\\s]+(/[^?#\\s]*)?$",
            "description": "The HTTPS URL serving the signed Plugin index (`plugin-index/index.json`) and its signature bundle, with no query, fragment or credentials.",
        })
    },
    examples: || vec![json!("https://plugins.example/plugin-index/index.json")],
};

fn default_plugin_index_url() -> Value {
    json!(DEFAULT_PLUGIN_INDEX_URL)
}

/// `plugin.index.url` (ADRs 0033 and 0043; issue 911).
///
/// Named under singular `plugin.*` (like `plugin.add`) rather than
/// `plugins.*` because `plugins` is already a map setting holding per-Plugin
/// entries (`plugins.<name>`), and no setting key may be a prefix of another.
pub const PLUGIN_INDEX_URL: Setting = Setting {
    name: "plugin.index.url",
    description: "The HTTPS URL serving the signed Plugin index (`plugin-index/index.json`), carrying its freshness `version`, 14-day expiry and revocation list. An Operator-configured endpoint (`egress.no_private_endpoints` refuses private addresses). Only Operator policy and user configuration set it; anywhere else it is `setting_not_applicable`.",
    kind: ValueKind::Shape(&PLUGIN_INDEX_URL_SHAPE),
    default: default_plugin_index_url,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The repository and workflow identity that signed a Plugin artifact or
/// index (ADR 0033).
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct SignerIdentity {
    /// The OIDC issuer that minted the signing identity, such as
    /// `https://token.actions.githubusercontent.com`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub issuer: String,
    /// The CI workflow reference that signed the artifact or index.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub workflow: String,
    /// The source repository that owns the workflow, such as
    /// `github.com/goodfetch/goodfetch`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub repository: String,
}

impl fmt::Display for SignerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} ({}, {})",
            self.repository, self.workflow, self.issuer
        )
    }
}

/// One conformance suite outcome recorded for a Plugin in the index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConformanceResult {
    /// The conformance suite name, such as `extract`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub suite: String,
    /// Whether the Plugin passed the suite.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub passed: bool,
    /// The engine version the suite ran against.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub engine_version: String,
}

/// One Plugin listed in the Plugin index (ADR 0033).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its OCI reference without digest, such as
    /// `ghcr.io/goodfetch/plugins/kettle`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The digest of its latest multi-platform OCI image index,
    /// `sha256:<hex>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The Plugin kinds it implements.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kinds: Vec<PluginKindName>,
    /// The repository and CI workflow identity expected to sign its
    /// provenance attestation.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: SignerIdentity,
    /// Whether it is first-party, third-party or a Private component.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub origin: Origin,
    /// The conformance suite results CI recorded for this release.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub conformance: Vec<ConformanceResult>,
}

/// One entry of the Plugin index's revocation list (ADR 0033): revokes a
/// specific artifact digest, a signer identity, or both.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Revocation {
    /// The revoked multi-platform OCI image index digest, `sha256:<hex>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: Option<String>,
    /// A revoked signer identity: revokes every Plugin attested by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: Option<SignerIdentity>,
    /// Why the digest or signer was revoked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// When the revocation was issued, in RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revoked_at: String,
}

impl Revocation {
    /// Whether this revocation matches `digest` or `signer`.
    #[must_use]
    pub fn matches(&self, digest: Option<&str>, signer: Option<&SignerIdentity>) -> bool {
        let digest_hit = self
            .digest
            .as_deref()
            .is_some_and(|revoked| digest == Some(revoked));
        let signer_hit = self
            .signer
            .as_ref()
            .is_some_and(|revoked| signer == Some(revoked));
        digest_hit || signer_hit
    }
}

/// The Plugin index document (`plugin-index/index.json`, ADRs 0033, 0052
/// and 0066): no `major.minor` format version, carrying only its monotonic
/// freshness `version`, signing and expiry timestamps, listed Plugins and
/// revocation list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginIndex {
    /// Monotonic freshness version: an index with an older version than the
    /// last seen is refused as stale.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: u64,
    /// When CI signed this index, in RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signed_at: String,
    /// When this index expires (14 days after signing), in RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expires_at: String,
    /// The Plugins currently listed in the index. Delisting removes an entry
    /// here without adding it to `revocations`.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugins: Vec<IndexEntry>,
    /// Revoked digests and signer identities.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revocations: Vec<Revocation>,
}

impl PluginIndex {
    /// The first revocation matching `digest` or `signer`, if any.
    #[must_use]
    pub fn revocation_for(
        &self,
        digest: Option<&str>,
        signer: Option<&SignerIdentity>,
    ) -> Option<&Revocation> {
        self.revocations
            .iter()
            .find(|revocation| revocation.matches(digest, signer))
    }

    /// The listed entry matching `name`, if any.
    #[must_use]
    pub fn entry(&self, name: &str) -> Option<&IndexEntry> {
        self.plugins.iter().find(|entry| entry.name == name)
    }

    /// The [`Origin`] of a Plugin named `name` with `digest`: first-party
    /// only when the index lists it as [`Origin::FirstParty`] at `digest`,
    /// Private when listed as [`Origin::Private`] at `digest`, and
    /// [`Origin::ThirdParty`] otherwise.
    #[must_use]
    pub fn origin_of(&self, name: &str, digest: Option<&str>) -> Origin {
        let Some(digest) = digest else {
            return Origin::ThirdParty;
        };
        self.plugins
            .iter()
            .find(|entry| entry.name == name && entry.digest == digest)
            .map_or(Origin::ThirdParty, |entry| entry.origin)
    }
}

/// The signature bundle carried alongside a signed set (Plugin index or
/// Clean-up rules set, ADRs 0033 and 0047).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignatureBundle {
    /// Identifier of the verifying key in the trust root.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key_id: String,
    /// OIDC issuer bound into the signature.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub issuer: String,
    /// Repository workflow reference bound into the signature.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub workflow: String,
    /// Base64-encoded Ed25519 signature over the domain-separated canonical
    /// JSON payload and signer claims.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signature: String,
}

/// A signed, versioned, expiring set together with its [`SignatureBundle`]
/// (ADRs 0033 and 0047).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedBundle<T> {
    /// The signed document.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: T,
    /// The signature bundle verifying `payload`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bundle: SignatureBundle,
}

/// The statement a Plugin artifact's provenance attestation signs (ADR 0033,
/// issue 916): binds an OCI multi-platform image index digest to the
/// repository and CI workflow identity that built it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceStatement {
    /// The OCI multi-platform image index digest (`sha256:<hex>`) this
    /// attestation covers.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub subject_digest: String,
    /// The repository, CI workflow and OIDC issuer identity that built and
    /// signed the artifact.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: SignerIdentity,
}

/// A signed provenance attestation attached to a Plugin OCI artifact via the
/// OCI 1.1 Referrers API (ADR 0033, issue 916).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceAttestation {
    /// The attested statement binding subject digest and signer identity.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub statement: ProvenanceStatement,
    /// Identifier of the verifying key in the attestation trust root.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key_id: String,
    /// Base64-encoded Ed25519 signature over the domain-separated RFC 8785
    /// canonical JSON of [`Self::statement`].
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signature: String,
}

/// Why a Plugin index was refused or blocked third-party loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IndexRefusalReason {
    /// Its signature did not verify under the trust root.
    BadSignature,
    /// Its signer identity did not match the trust root's workflow and issuer.
    UntrustedSigner,
    /// Its `version` is older than the last version seen.
    Stale,
    /// Its `expires_at` timestamp has passed.
    Expired,
}

/// A Plugin index refused for a bad signature, stale version or expiry
/// (ADR 0033).
pub const PLUGIN_INDEX_STALE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_index_stale",
    description: "The Plugin index failed signature verification, carried an older freshness `version` than the last one seen, or passed its 14-day expiry. An expired index blocks loading third-party Plugins because revocation can no longer be checked; first-party Plugins still load. Refresh the index or import a fresh signed one with `goodfetch plugin index import <file>`.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_index_stale` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginIndexStaleDetails {
    /// Why the index was refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: IndexRefusalReason,
    /// The index's `version`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: Option<u64>,
    /// The last freshness `version` previously seen, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub last_seen: Option<u64>,
    /// The index's `expires_at` timestamp in RFC 3339, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expires_at: Option<String>,
    /// What resolves the refusal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A Plugin whose digest or signer is on the index's revocation list
/// (ADR 0033).
pub const PLUGIN_REVOKED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "plugin_revoked",
    description: "The Plugin's digest or signer is on the Plugin index's revocation list: running instances are stopped at once, new starts are refused, and the revocation is recorded in the Audit log.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin_revoked` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginRevokedDetails {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugin: String,
    /// The revoked digest, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: Option<String>,
    /// Why the index revoked it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// What resolves the refusal.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A kind of Audit log entry: a value of the shared type `audit_kind`.
const fn audit_kind(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::AuditKind,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

/// A Plugin was revoked by the Plugin index.
pub const PLUGIN_REVOKED_AUDIT: SharedTypeValue = audit_kind(
    "plugin_revoked",
    "The Plugin index revoked a Plugin's digest or signer: running instances were stopped at once and new starts are refused, recording the Plugin, its digest, the revocation reason and the Run when stopped in flight.",
);

/// A signed Plugin index was imported offline.
pub const PLUGIN_INDEX_IMPORTED_AUDIT: SharedTypeValue = audit_kind(
    "plugin_index_imported",
    "A person imported a fresh signed Plugin index verified offline against the bundled trust root: its freshness version and expiry.",
);

/// What `plugin.search` takes (ADR 0033, issue 919).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginSearchParams {
    /// Optional case-insensitive text query matching a Plugin's name, kinds
    /// or signer identity; omitted lists every Plugin in the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub query: Option<String>,
    /// Optional filter restricting results to Plugins implementing this kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: Option<PluginKindName>,
}

/// What `plugin.search` answers (ADR 0033, issue 919).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginSearchResult {
    /// Freshness version of the cached Plugin index, when one has been
    /// loaded or imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub index_version: Option<u64>,
    /// RFC 3339 expiry timestamp of the cached Plugin index, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expires_at: Option<String>,
    /// Whether the cached Plugin index is past its expiry at the query
    /// instant.
    #[serde(default, skip_serializing_if = "is_false")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expired: bool,
    /// Matching Plugins from the index, each carrying its kinds, signer
    /// identity and conformance results.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugins: Vec<IndexEntry>,
}

/// `plugin.search` (ADR 0033, issue 919).
pub const PLUGIN_SEARCH: Operation = Operation {
    name: "plugin.search",
    description: "Searches the local signed Plugin index, returning each matching Plugin's name, OCI reference, pinned digest, kinds, signer identity, origin and conformance results so users choose a Plugin by what it has proved. Available on every surface.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PluginSearchParams>,
    result: schema_of::<PluginSearchResult>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `plugin.index_import` takes (ADR 0033, issue 919).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginIndexImportParams {
    /// The signed Plugin index bundle to verify offline against the release's
    /// trust root and persist in the Goodfetch home.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bundle: SignedBundle<PluginIndex>,
}

/// What `plugin.index_import` answers once the signed Plugin index is
/// verified offline, persisted and recorded in the Audit log (ADR 0033,
/// issue 919).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginIndexImported {
    /// The accepted index's monotonic freshness version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: u64,
    /// When CI signed the index, in RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signed_at: String,
    /// When the index expires, in RFC 3339.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expires_at: String,
    /// Number of Plugins listed in the imported index.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugins: u64,
    /// Number of revocation entries in the imported index.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revocations: u64,
}

/// `plugin.index_import` (ADR 0033, issue 919).
pub const PLUGIN_INDEX_IMPORT: Operation = Operation {
    name: "plugin.index_import",
    description: "Imports a fresh signed Plugin index verified offline against the trust root the release bundles, for air-gapped hosts so revocation works without network access. Refuses a badly signed, untrusted, stale or expired index with `plugin_index_stale` leaving the cached index untouched, and records accepted imports in the Audit log as `plugin_index_imported`. Person-only (ADR 0099): any non-person surface is refused with `person_required`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PluginIndexImportParams>,
    result: schema_of::<PluginIndexImported>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[PLUGIN_SEARCH, PLUGIN_INDEX_IMPORT],
    events: &[],
    handler_requests: &[],
    error_codes: &[PLUGIN_INDEX_STALE, PLUGIN_REVOKED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[PLUGIN_INDEX_URL],
    hook_stages: &[],
    shared_type_values: &[PLUGIN_REVOKED_AUDIT, PLUGIN_INDEX_IMPORTED_AUDIT],
};
