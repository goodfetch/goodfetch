//! Schemas in Rust's own types (ADR 0082, G7): how a value of one crosses
//! the wire. A [`Schema`] is a type deriving [`schemars::JsonSchema`] and
//! serde's traits, through the `schemars` this crate re-exports, or a JSON
//! Schema written by hand, the way down (G8), whose values are
//! [`serde_json::Value`]. The engine receives the schema's deserialize
//! side; a sent value is encoded by a checked serializer, which names the
//! JSON Pointer of the first value that is not JSON (`serde_json` turns NaN
//! into `null` and a key `1` into `"1"` silently), and must parse back as
//! itself; an answer is decoded by the type's own `Deserialize`. Serde has
//! one type for both sides.
//!
//! Each call names its own refusals (`value_invalid`, `payload_invalid`, a
//! Record's), so the adapter fails with its own [`SchemaError`], which the
//! call turns into its code. Python's `schema.py` and TypeScript's
//! `schema.ts` are its twins.

mod encode;
#[cfg(test)]
mod tests;

use std::fmt;
use std::marker::PhantomData;

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// How the values of one schema cross the wire: the JSON Schema the engine
/// receives, a sent value encoded as it parses back, and an answer decoded
/// to `T`. `T` is the program's type, or [`Value`] for a JSON Schema
/// written by hand.
pub struct Schema<T> {
    /// The JSON Schema the engine receives.
    wire: Value,
    values: PhantomData<fn(T) -> T>,
}

impl<T: JsonSchema + Serialize + DeserializeOwned> Schema<T> {
    /// The schema `schemars` derives for `T`, on the side `T` deserializes
    /// from (JSON Schema 2020-12): a field with a default is not required,
    /// as `Deserialize` reads it.
    ///
    /// # Examples
    ///
    /// ```
    /// use goodfetch_collie_client::Schema;
    /// use goodfetch_collie_client::schemars::{self, JsonSchema};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(JsonSchema, Serialize, Deserialize)]
    /// struct Book {
    ///     title: String,
    /// }
    ///
    /// let books = Schema::<Book>::derived();
    /// assert_eq!(books.wire_schema()["required"], serde_json::json!(["title"]));
    /// ```
    #[must_use]
    pub fn derived() -> Self {
        let wire = SchemaSettings::draft2020_12()
            .for_deserialize()
            .into_generator()
            .into_root_schema_for::<T>()
            .to_value();
        Self {
            wire,
            values: PhantomData,
        }
    }
}

impl Schema<Value> {
    /// A JSON Schema written by hand, the way down (G8), which depends on
    /// no `schemars` version: the engine checks values against it, and the
    /// program's values are [`Value`]s, which it narrows.
    ///
    /// # Errors
    ///
    /// [`SchemaError::NotASchema`] when `document` is neither an object
    /// nor a boolean, the two forms a JSON Schema takes.
    ///
    /// # Examples
    ///
    /// ```
    /// use goodfetch_collie_client::{Schema, SchemaError};
    /// use serde_json::json;
    ///
    /// let books = Schema::json(json!({"type": "object"}))?;
    /// assert_eq!(books.encode(&json!({"title": "Dune"}))?, json!({"title": "Dune"}));
    /// # Ok::<(), SchemaError>(())
    /// ```
    pub fn json(document: Value) -> Result<Self, SchemaError> {
        match document {
            Value::Object(_) | Value::Bool(_) => Ok(Self {
                wire: document,
                values: PhantomData,
            }),
            other => Err(SchemaError::NotASchema {
                found: encode::kind(&other).to_owned(),
            }),
        }
    }
}

impl<T: Serialize + DeserializeOwned> Schema<T> {
    /// The JSON Schema the engine receives: the side the schema parses
    /// from.
    #[must_use]
    pub fn wire_schema(&self) -> &Value {
        &self.wire
    }

    /// `value` as JSON, in the form the schema parses back.
    ///
    /// # Errors
    ///
    /// [`SchemaError::NotJson`], naming its JSON Pointer, when something
    /// in `value` is not JSON (NaN, an infinity, a map key that is not a
    /// string, an integer past 64 bits, a `Serialize` that fails);
    /// [`SchemaError::ChangesOnReadBack`], naming its JSON Pointer, when it
    /// reads back as another value; [`SchemaError::Rejected`] when `T`'s
    /// own `Deserialize` does not read its JSON back.
    ///
    /// # Examples
    ///
    /// ```
    /// use goodfetch_collie_client::{Schema, SchemaError};
    /// use serde_json::Value;
    /// use std::collections::BTreeMap;
    ///
    /// let prices = Schema::<BTreeMap<String, f64>>::derived();
    /// let refused = prices.encode(&BTreeMap::from([("dune".to_owned(), f64::NAN)]));
    /// assert!(matches!(refused, Err(SchemaError::NotJson { path, .. }) if path == "/dune"));
    /// ```
    pub fn encode(&self, value: &T) -> Result<Value, SchemaError> {
        encode_read_back(value)
    }

    /// `answer`, a value received, read by `T`'s own `Deserialize`.
    ///
    /// # Errors
    ///
    /// [`SchemaError::Rejected`] when `T` does not read it.
    ///
    /// # Examples
    ///
    /// ```
    /// use goodfetch_collie_client::{Schema, SchemaError};
    /// use std::collections::BTreeMap;
    ///
    /// let counts = Schema::<BTreeMap<String, u32>>::derived();
    /// let read = counts.decode(serde_json::json!({"pages": 3}))?;
    /// assert_eq!(read["pages"], 3);
    /// # Ok::<(), SchemaError>(())
    /// ```
    pub fn decode(&self, answer: Value) -> Result<T, SchemaError> {
        decode(answer)
    }
}

/// `value` as JSON, in the form `T`'s own `Deserialize` reads back: what a
/// [`Schema`] encodes, for a value sent with no schema of its own, such as
/// a Key-value store's.
///
/// # Errors
///
/// As [`Schema::encode`].
pub(crate) fn encode_read_back<T: Serialize + DeserializeOwned>(
    value: &T,
) -> Result<Value, SchemaError> {
    let encoded = encode::to_json(value)?;
    // What the engine reads parses back, and as the value sent: a field
    // `Deserialize` skips, or renames differently, is refused here.
    let parsed: T = decode(encoded.clone())?;
    let read_back = encode::to_json(&parsed)?;
    match encode::first_change(&encoded, &read_back) {
        None => Ok(encoded),
        Some((path, read_as)) => Err(SchemaError::ChangesOnReadBack { path, read_as }),
    }
}

/// `answer`, a value received, read by `T`'s own `Deserialize`.
///
/// # Errors
///
/// As [`Schema::decode`].
pub(crate) fn decode<T: DeserializeOwned>(answer: Value) -> Result<T, SchemaError> {
    serde_json::from_value(answer).map_err(|rejected| SchemaError::Rejected {
        message: rejected.to_string(),
    })
}

impl<T> Clone for Schema<T> {
    fn clone(&self) -> Self {
        Self {
            wire: self.wire.clone(),
            values: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Schema<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Schema")
            .field("type", &std::any::type_name::<T>())
            .field("wire", &self.wire)
            .finish()
    }
}

/// `value` as JSON, by the checked serializer a [`Schema`] encodes with,
/// for a value sent without a schema: what `serde_json` would turn into
/// `null` or a string key is refused instead.
///
/// # Errors
///
/// [`SchemaError::NotJson`], naming its JSON Pointer, when something in
/// `value` is not JSON (NaN, an infinity, a map key that is not a string,
/// an integer past 64 bits, a `Serialize` that fails).
///
/// # Examples
///
/// ```
/// use goodfetch_collie_client::{SchemaError, to_json};
///
/// let pages = to_json(&serde_json::json!({ "pages": 3 })).map(|json| json["pages"].clone());
/// assert_eq!(pages, Ok(serde_json::json!(3)));
/// assert!(matches!(to_json(&[1.0, f64::NAN]), Err(SchemaError::NotJson { path, .. }) if path == "/1"));
/// ```
pub fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<Value, SchemaError> {
    encode::to_json(value)
}

/// Why a [`Schema`] refused a value or a document.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// A value to send holds something that is not JSON (G3).
    #[error("{found} at {} is not JSON", at(path))]
    NotJson {
        /// Its JSON Pointer (RFC 6901), empty at the root.
        path: String,
        /// What it is: `NaN`, `the key 1, not a string`.
        found: String,
    },
    /// A value to send is JSON but parses back as another value: `T`'s
    /// `Deserialize` skips a field its `Serialize` writes, or reads it
    /// otherwise, so the engine would read what the program did not send.
    #[error("the value at {} reads back as {read_as}, not as sent", at(path))]
    ChangesOnReadBack {
        /// The JSON Pointer (RFC 6901) of the first value that changes,
        /// empty at the root.
        path: String,
        /// The JSON it reads back as.
        read_as: Value,
    },
    /// The type's own `Deserialize` rejected a value: one sent, as it
    /// parses back, or one received.
    #[error("the schema rejects the value: {message}")]
    Rejected {
        /// Serde's own message.
        message: String,
    },
    /// A JSON Schema written by hand is neither an object nor a boolean.
    #[error("a JSON Schema is an object or a boolean, not {found}")]
    NotASchema {
        /// What it is instead: `a number`, `a string`, `an array`, `null`.
        found: String,
    },
}

/// Where `path` points, as a message says it.
fn at(path: &str) -> &str {
    if path.is_empty() { "its root" } else { path }
}

/// C-SEND-SYNC: a schema moves between threads whatever its type holds,
/// since it holds none of it.
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Schema<std::rc::Rc<Value>>>();
    send_sync_static::<SchemaError>();
};
