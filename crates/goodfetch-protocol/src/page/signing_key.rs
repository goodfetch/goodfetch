//! An `identified` Session's Web Bot Auth key and its key directory (ADRs
//! 0021, 0045 and 0056; #1551): `signing_key.create` makes the Ed25519 key
//! in the OS keychain or an encrypted local file, kept as a Secret
//! reference, and `signing_key.directory` exports the signed key directory
//! the user hosts at `/.well-known/http-message-signatures-directory` of
//! the origin `identified.signature_agent` names, which a site fetches to
//! check each signed request. The CLI's commands are `signing-key create`
//! and `signing-key directory`.

use crate::point_in_time::PointInTime;
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ARRIVED_IN;
use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, Reach, Stability, schema_of,
};
use crate::secret_reference::REFERENCE_PATTERN;

/// The media type a key directory is served as.
pub const DIRECTORY_CONTENT_TYPE: &str = "application/http-message-signatures-directory+json";

/// Where on its origin a key directory is served.
pub const DIRECTORY_PATH: &str = "/.well-known/http-message-signatures-directory";

/// How long an exported directory's signatures stay valid when the call
/// names no time: seven days, in milliseconds.
pub const DEFAULT_VALID_FOR: u64 = 7 * 24 * 60 * 60 * 1000;

/// What `signing_key.create` takes: where the key is kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SigningKeyCreateParams {
    /// The Secret reference of the item the key is kept in, a `keychain:`
    /// or a `secretfile:` one, such as `keychain:goodfetch/signing-key`;
    /// the value `identified.signing_key` then takes.
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub signing_key: String,
}

/// A key directory's entry of one key: its public JSON Web Key, named by
/// its thumbprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PublicSigningKey {
    /// The key type, `OKP`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kty: String,
    /// The curve, `Ed25519`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crv: String,
    /// The public key, in unpadded Base64url.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub x: String,
    /// The key's ID, its RFC 7638 thumbprint, which each signature names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kid: String,
}

/// What `signing_key.create` answers: the key's reference and its public
/// half, never its private key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SigningKeyCreated {
    /// The Secret reference the key is kept at.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signing_key: String,
    /// The key's ID, its RFC 7638 thumbprint.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key_id: String,
    /// The key's public JSON Web Key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub public_key: PublicSigningKey,
    /// Whether this call made the key; false when the item held a key
    /// already, which is kept.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub created: bool,
}

/// `signing_key.create` (ADR 0045).
pub const SIGNING_KEY_CREATE: Operation = Operation {
    name: "signing_key.create",
    description: "Makes a new Ed25519 key for an `identified` Session's Web Bot Auth signatures, from the OS generator, and keeps it as a private JSON Web Key in the item a `keychain:` or `secretfile:` Secret reference names; answers the reference, the key's ID and its public key, never the private key. An item that holds an Ed25519 key already keeps it and answers it with `created` false; one that holds anything else is refused. Set `identified.signing_key` to the reference, then export the key directory with `signing_key.directory`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SigningKeyCreateParams>,
    result: schema_of::<SigningKeyCreated>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `signing_key.directory` takes: the keys it publishes and the
/// origin that serves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SigningKeyDirectoryParams {
    /// The Secret references of the keys the directory publishes, each
    /// proven by its own signature: the key in use, and while a rotation
    /// lasts the one before or after it.
    #[schemars(length(min = 1), inner(pattern(REFERENCE_PATTERN)), extend("arrived_in" = "0.1"))]
    pub signing_keys: Vec<String>,
    /// The authority of the origin that serves the directory, its host
    /// and its port when not the scheme's own, such as `bot.example`,
    /// which the signatures cover: the origin `identified.signature_agent`
    /// names.
    #[schemars(length(min = 1), extend("arrived_in" = "0.1"))]
    pub authority: String,
    /// How long the directory's signatures stay valid, in milliseconds,
    /// after which it is exported again; seven days when none is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1000), extend("arrived_in" = "0.1"))]
    pub valid_for: Option<u64>,
}

/// What `signing_key.directory` answers: the directory's body and the
/// header fields its response carries, served as they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SigningKeyDirectory {
    /// Where on the origin the directory is served.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// The response's `Content-Type`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content_type: String,
    /// The response's body, the JSON key set, byte for byte.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: String,
    /// The response's `Signature-Input` field, one label per key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signature_input: String,
    /// The response's `Signature` field, one label per key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signature: String,
    /// The IDs of the keys it publishes, in the order given.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key_ids: Vec<String>,
    /// When its signatures expire, by which it is exported again.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub expires: SystemTime,
}

/// `signing_key.directory` (ADR 0045).
pub const SIGNING_KEY_DIRECTORY: Operation = Operation {
    name: "signing_key.directory",
    description: "Exports the Web Bot Auth key directory of the keys the Secret references name, for the user to serve at `/.well-known/http-message-signatures-directory` of the origin `identified.signature_agent` names: the body, a JSON key set of each key's public JSON Web Key named by its RFC 7638 thumbprint, and the `Content-Type`, `Signature-Input` and `Signature` fields its response carries, where each key signs the origin's authority with the tag `http-message-signatures-directory` to prove it holds the key. Valid for `valid_for` milliseconds, seven days by default; export it again before `expires`. A reference that gives no Ed25519 private JSON Web Key is refused.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<SigningKeyDirectoryParams>,
    result: schema_of::<SigningKeyDirectory>,
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
    operations: &[SIGNING_KEY_CREATE, SIGNING_KEY_DIRECTORY],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
