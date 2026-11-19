//! A top-level result as the program reads it (ADR 0082): its typed view,
//! which ignores the fields this SDK does not know, as serde does, and the
//! JSON it came from, which keeps them, readable through [`Answer::raw`].
//! An engine newer than the SDK adds a field to a result; the program
//! reads it there before the SDK types it.

use std::ops::Deref;

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// A result the engine answered: `T`, read from the JSON it came from, which
/// [`raw`](Self::raw) keeps whole, the fields `T` does not name included.
/// It reads as `T` through [`Deref`].
///
/// # Examples
///
/// ```
/// use goodfetch_collie_client::Answer;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Usage {
///     pages: u64,
/// }
///
/// let usage: Answer<Usage> = serde_json::from_str(r#"{"pages": 3, "grams": 1.5}"#).unwrap();
/// assert_eq!(usage.pages, 3);
/// assert_eq!(usage.raw()["grams"], 1.5);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Answer<T> {
    typed: T,
    raw: Value,
}

impl<T> Answer<T> {
    /// The JSON the result came from, whole: the fields `T` does not name
    /// too.
    #[must_use]
    pub fn raw(&self) -> &Value {
        &self.raw
    }

    /// Its typed view, without the JSON it came from.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.typed
    }

    /// Its typed view and the JSON it came from.
    #[must_use]
    pub fn into_parts(self) -> (T, Value) {
        (self.typed, self.raw)
    }
}

impl<T> Deref for Answer<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.typed
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Answer<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let typed = T::deserialize(&raw).map_err(D::Error::custom)?;
        Ok(Self { typed, raw })
    }
}
