//! A Run's Key-value store (ADRs 0088, 0102 and 0103): every Run, a Collie
//! Run included, keeps JSON values by key in its run state, so a program
//! that resumes the Run, here or on another machine, finds them.
//! `key_value_store.get` answers a value or null, `key_value_store.set`
//! refuses null so that a null from `get` always means an absent key,
//! `key_value_store.delete` says whether the key existed, and
//! `key_value_store.keys` sends the keys in `$/key` notifications, one per
//! credit, in the order of their Unicode scalar values. Reads take no
//! writer lock, so they serve a view of a Run no program holds. A key is 1
//! to 512 Unicode scalar values with no control character; any other key,
//! and a null value, fail with `value_invalid`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::jsonrpc::RequestId;
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The most Unicode scalar values a key holds.
pub const KEY_MAX_SCALARS: usize = 512;

/// The notification that carries one key `key_value_store.keys` sends.
pub const KEY: &str = "$/key";

/// What `key_value_store.get` and `key_value_store.delete` take: the Run
/// and the key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyValueStoreKeyParams {
    /// The Run whose store it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
}

/// What `key_value_store.set` takes: the Run, the key and its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyValueStoreSetParams {
    /// The Run whose store it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The value: any JSON but a top-level null, which `value_invalid`
    /// refuses; a nested null is kept.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
}

/// What `key_value_store.keys` takes: the Run, and the prefix every key it
/// sends starts with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyValueStoreKeysParams {
    /// The Run whose store it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The prefix of the keys it sends; every key when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub prefix: Option<String>,
}

/// What `key_value_store.get` answers: the key's committed value, or null
/// when the key is absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyValueStoreValue {
    /// The value, or null for an absent key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: Value,
}

/// What `key_value_store.set` answers once the value is committed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyValueStoreSet {}

/// What `key_value_store.delete` answers once the key is gone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyValueStoreDeleted {
    /// Whether the key existed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub existed: bool,
}

/// What `key_value_store.keys` answers once it sent its last key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyValueStoreKeysEnded {
    /// How many keys it sent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: u64,
}

/// What `$/key` carries: the next key of the store a call lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyValueStoreKey {
    /// The id of the `key_value_store.keys` call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
}

/// What a `value_invalid` refused: the key itself, or the value given for
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Refused {
    /// The key.
    Key,
    /// The value.
    Value,
}

/// Why `value_invalid` refused a key or a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueInvalidReason {
    /// The key is empty.
    KeyEmpty,
    /// The key holds more than 512 Unicode scalar values.
    KeyTooLong,
    /// The key holds a control character.
    KeyControlCharacter,
    /// The value is null, which `get` answers for an absent key.
    ValueNull,
}

/// What `value_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValueInvalidDetails {
    /// The key, as the call gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Whether the key itself or its value was refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub refused: Refused,
    /// Why.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: ValueInvalidReason,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A key or a value the Key-value store refuses (ADR 0103).
pub const VALUE_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "value_invalid",
    description: "The Key-value store refused the key or its value: a key must be 1 to 512 Unicode scalar values with no control character, and a value any JSON but null, which `get` answers for an absent key; remove a key with `delete`. The details name the key and say which was refused.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.get`.
pub const KEY_VALUE_STORE_GET: Operation = Operation {
    name: "key_value_store.get",
    description: "Answers the value a Run's Key-value store holds for a key, as committed, or null when the key is absent. Takes no writer lock, so it reads a Run no program holds without resuming it.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreKeyParams>,
    result: schema_of::<KeyValueStoreValue>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.set`.
pub const KEY_VALUE_STORE_SET: Operation = Operation {
    name: "key_value_store.set",
    description: "Commits a JSON value for a key in a live Run's Key-value store, replacing the one it held: the last commit wins. A null value fails with `value_invalid`; remove a key with `delete`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreSetParams>,
    result: schema_of::<KeyValueStoreSet>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.delete`.
pub const KEY_VALUE_STORE_DELETE: Operation = Operation {
    name: "key_value_store.delete",
    description: "Removes a key from a live Run's Key-value store and answers whether it existed.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreKeyParams>,
    result: schema_of::<KeyValueStoreDeleted>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.keys`.
pub const KEY_VALUE_STORE_KEYS: Operation = Operation {
    name: "key_value_store.keys",
    description: "Sends the committed keys of a Run's Key-value store that start with `prefix`, in the order of their Unicode scalar values, in `$/key` notifications, one per credit granted with `$/credit`, then answers how many it sent. Takes no writer lock.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreKeysParams>,
    result: schema_of::<KeyValueStoreKeysEnded>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[
        KEY_VALUE_STORE_GET,
        KEY_VALUE_STORE_SET,
        KEY_VALUE_STORE_DELETE,
        KEY_VALUE_STORE_KEYS,
    ],
    events: &[],
    handler_requests: &[],
    error_codes: &[VALUE_INVALID],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
