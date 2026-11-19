//! A Run's Key-value store (ADRs 0088, 0102 and 0103): every Run, a Collie
//! Run included, keeps JSON values, and bytes with their content type, by
//! key in its run state, so a program that resumes the Run, here or on
//! another machine, finds them. `key_value_store.get` answers a value or
//! null, `key_value_store.set` refuses null so that a null from `get`
//! always means an absent key, `key_value_store.get_bytes` and
//! `key_value_store.set_bytes` read and write bytes, and neither reader
//! answers the other's type (S2): `get` on bytes and `get_bytes` on JSON
//! fail with `value_invalid` naming the other,
//! `key_value_store.delete` says whether the key existed, and
//! `key_value_store.keys` sends the keys in `$/key` notifications, one per
//! credit, in the order of their Unicode scalar values. Reads take no
//! writer lock, so they serve a view of a Run no program holds. A key is 1
//! to 512 Unicode scalar values with no control character; any other key,
//! and a null value, fail with `value_invalid`.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
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
    /// The crawl attempt whose handler's `ctx.run` reads or deletes the
    /// key: a read sees what the attempt staged, and a delete is staged
    /// with the attempt (ADR 0093). Absent, the call reads or commits at
    /// once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: Option<KeyValueStoreAttempt>,
}

/// The crawl attempt a Key-value store call is made through: a handler's
/// `ctx.run`, which is bound to its attempt (ADR 0093). Only a crawl's Run
/// has attempts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyValueStoreAttempt {
    /// The ID of the crawl request whose attempt makes the call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request_id: String,
    /// The attempt's lease epoch, as `crawl_request.handle` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lease_epoch: u64,
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
    /// The crawl attempt whose handler's `ctx.run` sets the key: the write
    /// is staged with the attempt and commits with it, or is discarded
    /// with it (ADR 0093). Absent, the value is committed at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: Option<KeyValueStoreAttempt>,
}

/// Bytes the store keeps: on the wire, in Base64 (RFC 4648, with padding).
#[derive(Clone, PartialEq, Eq)]
pub struct KeyValueStoreContent(Vec<u8>);

/// Why a text is no content: it is no Base64 with padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("content is its bytes in Base64 (RFC 4648, with padding)")]
pub struct KeyValueStoreContentInvalid;

impl KeyValueStoreContent {
    /// The content `bytes`.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Reads `text`, bytes in Base64.
    ///
    /// # Errors
    ///
    /// When `text` is no Base64 with padding.
    pub fn from_base64(text: &str) -> Result<Self, KeyValueStoreContentInvalid> {
        data_encoding::BASE64
            .decode(text.as_bytes())
            .map(Self)
            .map_err(|_| KeyValueStoreContentInvalid)
    }

    /// Its bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// Its bytes, owned.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl std::fmt::Debug for KeyValueStoreContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "KeyValueStoreContent({} bytes)", self.0.len())
    }
}

impl Serialize for KeyValueStoreContent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&data_encoding::BASE64.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for KeyValueStoreContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::from_base64(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for KeyValueStoreContent {
    fn schema_name() -> Cow<'static, str> {
        "KeyValueStoreContent".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Bytes the Key-value store keeps, in Base64 (RFC 4648, with padding).",
            "type": "string",
            "contentEncoding": "base64",
        })
    }
}

/// What `key_value_store.set_bytes` takes: the Run, the key, and the bytes
/// with their content type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyValueStoreSetBytesParams {
    /// The Run whose store it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The bytes, in Base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content: KeyValueStoreContent,
    /// Their content type, such as `image/png`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content_type: String,
    /// The crawl attempt whose handler's `ctx.run` sets the bytes: the
    /// write is staged with the attempt and commits with it, a kept file
    /// written only then, or is discarded with it (ADR 0093). Absent, the
    /// bytes are committed at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: Option<KeyValueStoreAttempt>,
}

/// Bytes the store holds for a key, with their content type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyValueStoreBytes {
    /// The bytes, in Base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content: KeyValueStoreContent,
    /// Their content type, as `set_bytes` gave it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content_type: String,
}

/// What `key_value_store.get_bytes` answers: the key's committed bytes, or
/// null when the key is absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyValueStoreBytesValue {
    /// The bytes and their content type, or null for an absent key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bytes: Option<KeyValueStoreBytes>,
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
    /// The key holds bytes, which `get_bytes` reads, not `get`.
    ValueBytes,
    /// The key holds JSON, which `get` reads, not `get_bytes`.
    ValueJson,
    /// The value given holds something that is not JSON, at `path`: the
    /// SDK refuses it before sending (ADR 0082).
    ValueNotJson,
    /// The value given, or the one stored, is rejected by the schema the
    /// call named, first at `path`: the SDK's own check (ADR 0082).
    ValueSchema,
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
    /// Where in the value the refusal is, a JSON Pointer (RFC 6901), for
    /// `value_not_json` and `value_schema`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// A key or a value the Key-value store refuses (ADR 0103).
pub const VALUE_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "value_invalid",
    description: "The Key-value store refused the key or its value: a key must be 1 to 512 Unicode scalar values with no control character, and a value any JSON but null, which `get` answers for an absent key; remove a key with `delete`. Bytes are read with `getBytes` and JSON with `get`, never the other way. The SDK refuses a value that is not JSON before sending it, and a value the call's schema rejects, sent or read. The details name the key, say which was refused and, for a value's content, its path as a JSON Pointer.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.get`.
pub const KEY_VALUE_STORE_GET: Operation = Operation {
    name: "key_value_store.get",
    description: "Answers the JSON value a Run's Key-value store holds for a key, as committed, or null when the key is absent. A key holding bytes fails with `value_invalid`, naming `getBytes`. Takes no writer lock, so it reads a Run no program holds without resuming it. Through a crawl attempt (`attempt`), it reads what the attempt staged first.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreKeyParams>,
    result: schema_of::<KeyValueStoreValue>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.set`.
pub const KEY_VALUE_STORE_SET: Operation = Operation {
    name: "key_value_store.set",
    description: "Commits a JSON value for a key in a live Run's Key-value store, replacing the one it held: the last commit wins. A null value fails with `value_invalid`; remove a key with `delete`. Through a crawl attempt (`attempt`), the write is staged with the attempt and commits or is discarded with it; once the attempt ended, it fails with `attempt_ended`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreSetParams>,
    result: schema_of::<KeyValueStoreSet>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.get_bytes`.
pub const KEY_VALUE_STORE_GET_BYTES: Operation = Operation {
    name: "key_value_store.get_bytes",
    description: "Answers the bytes and content type a Run's Key-value store holds for a key, as committed, or null when the key is absent. A key holding JSON fails with `value_invalid`, naming `get`. Takes no writer lock. Through a crawl attempt (`attempt`), it reads what the attempt staged first.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreKeyParams>,
    result: schema_of::<KeyValueStoreBytesValue>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.set_bytes`.
pub const KEY_VALUE_STORE_SET_BYTES: Operation = Operation {
    name: "key_value_store.set_bytes",
    description: "Commits bytes with their content type for a key in a live Run's Key-value store, replacing what it held: the last commit wins. Bytes over 1 MiB are kept as a file in the Run's folder, which its Checkpoints carry.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreSetBytesParams>,
    result: schema_of::<KeyValueStoreSet>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `key_value_store.delete`.
pub const KEY_VALUE_STORE_DELETE: Operation = Operation {
    name: "key_value_store.delete",
    description: "Removes a key from a live Run's Key-value store and answers whether it existed. Through a crawl attempt (`attempt`), the removal is staged with the attempt, and whether the key existed counts what the attempt staged.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<KeyValueStoreKeyParams>,
    result: schema_of::<KeyValueStoreDeleted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
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
    input: None,
    bound: None,
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
        KEY_VALUE_STORE_GET_BYTES,
        KEY_VALUE_STORE_SET_BYTES,
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
