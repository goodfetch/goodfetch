//! What resolving a Secret reference answers (ADRs 0015, 0092 and 0129):
//! the Error codes a resolution fails with, each naming the placeholder and
//! the reference and never a value, and the settings the Secret-provider
//! driver reads. The references' grammar is `secret_reference`'s.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// What a resolution failed on, which `secret_unavailable` carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretFailure {
    /// The provider holds no value for the reference, such as an unset
    /// variable.
    Missing,
    /// The engine has no such provider, or its `secret_providers.<scheme>.*`
    /// configuration does not let it answer.
    NotConfigured,
    /// The provider failed in a way the engine does not know.
    ProviderFailed,
    /// The caller's `resolveSecret` is missing, threw or gave no answer.
    ResolverFailed,
    /// The provider gave no answer within `timeout.secret`.
    TimedOut,
    /// No one-time code came: a provider made none from the reference,
    /// such as a TOTP seed it cannot read, or a code source's code never
    /// arrived (ADR 0092).
    NoCode,
}

impl SecretFailure {
    /// Every value, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Missing,
        Self::NotConfigured,
        Self::ProviderFailed,
        Self::ResolverFailed,
        Self::TimedOut,
        Self::NoCode,
    ];

    /// Its name as the wire writes it.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// A provider gave no value (ADR 0092).
pub const SECRET_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "secret_unavailable",
    description: "A Secret provider gave no value for a reference: a missing variable, a locked keychain, an unreachable vault, no one-time code (a TOTP seed the provider makes none from, or a code source's code that never arrived), the caller's `resolveSecret` missing or throwing, no answer within `timeout.secret`. `failure` says which, and the details name the placeholder or setting, the reference and the provider, never a value.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `secret_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretUnavailableDetails {
    /// The Secret placeholder the value was for, or the setting's key where
    /// no placeholder names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
    /// The Secret reference.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The provider, by its scheme, such as `env`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub provider: String,
    /// What failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failure: SecretFailure,
    /// What makes the reference resolve.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// No scope binds the placeholder (ADR 0092).
pub const SECRET_UNBOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "secret_unbound",
    description: "No scope binds the Secret placeholder a call names. `bound` lists the placeholders bound, never a value.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `secret_unbound` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretUnboundDetails {
    /// The placeholder no scope binds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
    /// The placeholders the scopes bind, never a value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bound: Vec<String>,
}

/// The receiving domain is outside the binding's (ADR 0092).
pub const SECRET_DOMAIN_NOT_ALLOWED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "secret_domain_not_allowed",
    description: "A Secret placeholder's value would go to a domain outside its binding's `domains`: the field's frame or the request's URL. The details name the placeholder, the reference and the domain, never a value.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `secret_domain_not_allowed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretDomainNotAllowedDetails {
    /// The placeholder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
    /// The Secret reference.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The host the value would have gone to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// The binding's domains.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domains: Vec<String>,
}

/// A value would go where it must not (ADR 0092).
pub const SECRET_INPUT_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "secret_input_refused",
    description: "A Secret placeholder's value would be typed or sent where other code may read it, or to a host reached through an invalid-certificate exception, so it was not resolved. The details name the placeholder, the reference, the domain and the `reason`, never a value.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Why a value may not go where it would.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretInputRefusedReason {
    /// The receiving host was reached through an invalid-certificate
    /// exception (ADR 0042), so whoever holds its key may not be the site.
    InvalidCertificateException,
}

impl SecretInputRefusedReason {
    /// Every value, in declaration order.
    pub const ALL: [Self; 1] = [Self::InvalidCertificateException];

    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// What `secret_input_refused` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretInputRefusedDetails {
    /// The placeholder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placeholder: String,
    /// The Secret reference.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The host the value would have gone to.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: String,
    /// Why it may not go there.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: SecretInputRefusedReason,
}

/// A Run's data key does not open under the key-encryption key a resume
/// named (ADR 0015).
pub const RUN_KEY_MISMATCH: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_key_mismatch",
    description: "A resumed Run's data key does not open under the key-encryption key `run_state.key_reference` names, such as a Checkpoint moved to a machine whose own key is another: the key that wrapped it never travels with the Checkpoint. The details name the Run, the setting, the reference resolved and the reference that wrapped the key, never a value. Name the key that wrapped it, in project configuration or configuration, and resume again.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `run_key_mismatch` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunKeyMismatchDetails {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The setting that names the key-encryption key,
    /// `run_state.key_reference`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub setting: String,
    /// The Secret reference the resume resolved.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reference: String,
    /// The Secret reference of the key that wrapped the data key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub wrapped_by: String,
    /// What makes the resume open it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

fn thirty_seconds() -> Value {
    json!(30_000)
}

/// `run_state.key_reference`'s default where the host has an OS keychain.
pub const KEY_REFERENCE_IN_KEYCHAIN: &str = "keychain:goodfetch/run-state-key";

/// `run_state.key_reference`'s default where the host has no OS keychain:
/// the encrypted local file.
pub const KEY_REFERENCE_IN_FILE: &str = "secretfile:run-state-key";

fn in_keychain() -> Value {
    json!(KEY_REFERENCE_IN_KEYCHAIN)
}

/// `run_state.key_reference` (ADR 0015).
pub const RUN_STATE_KEY_REFERENCE: Setting = Setting {
    name: "run_state.key_reference",
    description: "The Secret reference of the key-encryption key that wraps each Run's data key. One set in a layer is resolved when a Run starts: a Run whose reference does not resolve is refused with `secret_unavailable`. Taken only from an Operator policy and user and project configuration. By default the OS keychain's `keychain:goodfetch/run-state-key`, or, on a host with no OS keychain, the encrypted local file's `secretfile:run-state-key`, which `config explain` shows as the default it chose; the engine makes the default's item, a random 256-bit key, where it is missing when a Run first seals a field. A Run's data key is stored only wrapped by this key, so a resume, such as of a Checkpoint moved to another machine, unwraps it with the key this setting names for the resume, which a Client may send in `run.resume`'s project configuration: one that gives no value fails the resume with `secret_unavailable`, one that does not open the data key with `run_key_mismatch`. The encrypted local file, `secrets/<name>.age` in Goodfetch home encrypted with age to the X25519 identity in `secrets/identity.txt`, a file only its owner may read, protects copies and backups of the home, not the value from another process of the same user.",
    kind: ValueKind::SecretReference,
    default: in_keychain,
    clearable: true,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `timeout.secret` (ADR 0092).
pub const TIMEOUT_SECRET: Setting = Setting {
    name: "timeout.secret",
    description: "How long a Secret provider may take to answer one reference before the resolution fails with `secret_unavailable`, `failure: \"timed_out\"`: 30 s. A provider that answers it is briefly busy, such as a rate limit's, is asked again within it, with exponential backoff and full jitter, after its `Retry-After` when it gave one.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// No command, or no address: the provider is not configured.
fn no_command() -> Value {
    Value::Null
}

/// `secret_providers.op.command` (spec #564): the 1Password CLI that
/// answers `op://` references.
pub const SECRET_PROVIDERS_OP_COMMAND: Setting = Setting {
    name: "secret_providers.op.command",
    description: "The absolute path of the 1Password CLI, `op`, that answers `op://vault/item/field` Secret references: the engine runs `op read` with its own environment, so `OP_SERVICE_ACCOUNT_TOKEN` or the desktop app's integration signs it in. Unset, the default, an `op://` reference fails with `secret_unavailable`, `failure: \"not_configured\"`. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: no_command,
    clearable: true,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `secret_providers.bw.command` (spec #564): the Bitwarden CLI that
/// answers `bw:` references, from Bitwarden or a Vaultwarden server.
pub const SECRET_PROVIDERS_BW_COMMAND: Setting = Setting {
    name: "secret_providers.bw.command",
    description: "The absolute path of the Bitwarden CLI, `bw`, that answers `bw:item/field` Secret references from Bitwarden or, once `bw config server` points it there, a Vaultwarden server: the engine runs `bw get` without interaction and with its own environment, so `BW_SESSION` unlocks the vault. `field` is `password`, `username`, `totp` (the current code, never the seed, or `failure: \"no_code\"` when Bitwarden makes none from it), `notes`, `uri` or the name of a custom field. Unset, the default, a `bw:` reference fails with `secret_unavailable`, `failure: \"not_configured\"`. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: no_command,
    clearable: true,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `secret_providers.vault.address` (spec #564): the Vault or `OpenBao`
/// server that answers `vault:` references, an Operator-configured
/// endpoint.
pub const SECRET_PROVIDERS_VAULT_ADDRESS: Setting = Setting {
    name: "secret_providers.vault.address",
    description: "The URL of the HashiCorp Vault or OpenBao server that answers `vault:mount/path#field` Secret references, such as `https://vault.example.com:8200`: the engine reads `field` of the KV version 2 secret `path` in the secrets engine mounted at `mount`, with the token in its environment's `VAULT_TOKEN`, or `BAO_TOKEN`. It is an Operator-configured endpoint: reached on a private address unless `egress.no_private_endpoints` refuses it, its certificate verified against Mozilla's roots and its own domain's `tls.trusted_cas`, never excepted by `tls.accept_invalid_certs`. Plain `http:` is taken only for a loopback address, as a development server's. Unset, the default, a `vault:` reference fails with `secret_unavailable`, `failure: \"not_configured\"`. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: no_command,
    clearable: true,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The public Secret Manager API, `secret_providers.gcp_sm.endpoint`'s
/// default.
fn gcp_secret_manager() -> Value {
    json!("https://secretmanager.googleapis.com")
}

/// Each vault's public URL, `secret_providers.azure_kv.endpoint`'s default.
fn azure_key_vault() -> Value {
    json!("https://{vault}.vault.azure.net")
}

/// Microsoft Entra ID's public authority,
/// `secret_providers.azure_kv.authority`'s default.
fn microsoft_entra_id() -> Value {
    json!("https://login.microsoftonline.com")
}

/// `secret_providers.aws_sm.region` (spec #564): the AWS Region whose
/// Secrets Manager answers `aws-sm:` references.
pub const SECRET_PROVIDERS_AWS_SM_REGION: Setting = Setting {
    name: "secret_providers.aws_sm.region",
    description: "The AWS Region, such as `eu-west-3`, whose Secrets Manager answers `aws-sm:name#key` Secret references: the engine calls `GetSecretValue` for the secret `name`, signed with the access key in its environment's `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and, for temporary credentials, `AWS_SESSION_TOKEN`, and answers the secret's text, or its JSON object's `key`. Unset, the default, the engine's environment's `AWS_REGION`, then `AWS_DEFAULT_REGION`, names it; with none, an `aws-sm:` reference fails with `secret_unavailable`, `failure: \"not_configured\"`. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: no_command,
    clearable: true,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `secret_providers.aws_sm.endpoint` (spec #564): the Secrets Manager
/// endpoint `aws-sm:` references are read from, an Operator-configured
/// endpoint.
pub const SECRET_PROVIDERS_AWS_SM_ENDPOINT: Setting = Setting {
    name: "secret_providers.aws_sm.endpoint",
    description: "The URL of the Secrets Manager endpoint that answers `aws-sm:` Secret references, such as a VPC endpoint's: unset, the default, the Region's public endpoint, `https://secretsmanager.<region>.amazonaws.com`. It is an Operator-configured endpoint: reached on a private address unless `egress.no_private_endpoints` refuses it, its certificate verified against Mozilla's roots and its own domain's `tls.trusted_cas`, never excepted by `tls.accept_invalid_certs`. Plain `http:` is taken only for a loopback address. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: no_command,
    clearable: true,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `secret_providers.gcp_sm.endpoint` (spec #564): the Secret Manager API
/// `gcp-sm:` references are read from, an Operator-configured endpoint.
pub const SECRET_PROVIDERS_GCP_SM_ENDPOINT: Setting = Setting {
    name: "secret_providers.gcp_sm.endpoint",
    description: "The URL of the Google Cloud Secret Manager API that answers `gcp-sm:project/secret/version` Secret references, `https://secretmanager.googleapis.com` by default, or a regional or Private Service Connect endpoint: the engine accesses the secret's version (`latest` for the newest) with the OAuth 2.0 access token in its environment's `GOOGLE_OAUTH_ACCESS_TOKEN`, such as `gcloud auth print-access-token` prints, and answers its payload as text. With no token, a `gcp-sm:` reference fails with `secret_unavailable`, `failure: \"not_configured\"`. It is an Operator-configured endpoint: reached on a private address unless `egress.no_private_endpoints` refuses it, its certificate verified against Mozilla's roots and its own domain's `tls.trusted_cas`, never excepted by `tls.accept_invalid_certs`. Plain `http:` is taken only for a loopback address. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: gcp_secret_manager,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `secret_providers.azure_kv.endpoint` (spec #564): the URL of each Key
/// Vault `azure-kv:` references are read from, an Operator-configured
/// endpoint.
pub const SECRET_PROVIDERS_AZURE_KV_ENDPOINT: Setting = Setting {
    name: "secret_providers.azure_kv.endpoint",
    description: "The URL of the Azure Key Vault that answers `azure-kv:vault/secret` Secret references, `{vault}` standing for the reference's vault name: `https://{vault}.vault.azure.net` by default, or a sovereign cloud's, such as `https://{vault}.vault.azure.cn`. The engine reads the secret's current version with an access token for Key Vault that Microsoft Entra ID (`secret_providers.azure_kv.authority`) issues to the service principal in its environment's `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET`. With any of them unset, an `azure-kv:` reference fails with `secret_unavailable`, `failure: \"not_configured\"`. It is an Operator-configured endpoint: reached on a private address unless `egress.no_private_endpoints` refuses it, its certificate verified against Mozilla's roots and its own domain's `tls.trusted_cas`, never excepted by `tls.accept_invalid_certs`. Plain `http:` is taken only for a loopback address. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: azure_key_vault,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `secret_providers.azure_kv.authority` (spec #564): the Microsoft Entra
/// ID authority that issues the Key Vault access token, an
/// Operator-configured endpoint.
pub const SECRET_PROVIDERS_AZURE_KV_AUTHORITY: Setting = Setting {
    name: "secret_providers.azure_kv.authority",
    description: "The URL of the Microsoft Entra ID authority that issues the access token `azure-kv:` references are read with, `https://login.microsoftonline.com` by default, or a sovereign cloud's, such as `https://login.chinacloudapi.cn`: the engine asks `<authority>/<AZURE_TENANT_ID>/oauth2/v2.0/token` for a token of the client-credentials grant. It is an Operator-configured endpoint, judged and verified as `secret_providers.azure_kv.endpoint` is. Taken only from an Operator policy and user and project configuration.",
    kind: ValueKind::Text,
    default: microsoft_entra_id,
    clearable: false,
    authority: Authority::Configuration,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn no_reference() -> Value {
    json!([])
}

/// `secrets.resolve_each_use` (ADR 0014's note of 2026-10-28).
pub const SECRETS_RESOLVE_EACH_USE: Setting = Setting {
    name: "secrets.resolve_each_use",
    description: "The Secret references a Run resolves again at each use, such as a rotating token's. Any other reference is resolved once for each placeholder it fills and its value reused for the rest of the Run, its domain binding checked at each use; none by default.",
    kind: ValueKind::List(&ValueKind::SecretReference),
    default: no_reference,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        SECRET_UNAVAILABLE,
        SECRET_UNBOUND,
        SECRET_DOMAIN_NOT_ALLOWED,
        SECRET_INPUT_REFUSED,
        RUN_KEY_MISMATCH,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        RUN_STATE_KEY_REFERENCE,
        TIMEOUT_SECRET,
        SECRETS_RESOLVE_EACH_USE,
        SECRET_PROVIDERS_OP_COMMAND,
        SECRET_PROVIDERS_BW_COMMAND,
        SECRET_PROVIDERS_VAULT_ADDRESS,
        SECRET_PROVIDERS_AWS_SM_REGION,
        SECRET_PROVIDERS_AWS_SM_ENDPOINT,
        SECRET_PROVIDERS_GCP_SM_ENDPOINT,
        SECRET_PROVIDERS_AZURE_KV_ENDPOINT,
        SECRET_PROVIDERS_AZURE_KV_AUTHORITY,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
