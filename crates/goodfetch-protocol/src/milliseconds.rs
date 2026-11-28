//! Durations on the wire (ADR 0080): a field the schema declares as
//! [`Milliseconds`] is a [`Duration`] in Rust, a `timedelta` in Python and a
//! `number` of milliseconds in TypeScript, each language's one duration
//! type, and crosses the wire as whole milliseconds. A field takes
//! `#[serde(with = "…::milliseconds")]` and, for its schema,
//! `#[schemars(with = "Milliseconds")]`.

use std::borrow::Cow;
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serializer};

/// The schema of a duration, the `$defs` entry each duration field refers
/// to; the value itself is a [`Duration`].
pub enum Milliseconds {}

impl JsonSchema for Milliseconds {
    fn schema_name() -> Cow<'static, str> {
        "Milliseconds".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A duration, in milliseconds.",
            "type": "integer",
            "format": "uint64",
            "minimum": 0,
        })
    }
}

/// Writes `duration` as its whole milliseconds, dropping any part of one.
///
/// # Errors
///
/// When it holds more milliseconds than a `u64` counts, or the
/// serializer's.
pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    let millis = u64::try_from(duration.as_millis()).map_err(|_| {
        serde::ser::Error::custom(format!("{duration:?} is past what the wire counts"))
    })?;
    serializer.serialize_u64(millis)
}

/// Reads a duration from its milliseconds.
///
/// # Errors
///
/// The deserializer's, as for a number that is no `u64`.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    u64::deserialize(deserializer).map(Duration::from_millis)
}

/// A nullable duration in milliseconds: `null` is `None`. A field that may be
/// left out takes `#[serde(default)]` too.
pub mod nullable {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes `duration` as whole milliseconds, or `null`.
    ///
    /// # Errors
    ///
    /// As [`super::serialize`].
    pub fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(duration) => super::serialize(duration, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Reads a duration in milliseconds, or `null` as `None`.
    ///
    /// # Errors
    ///
    /// As [`super::deserialize`].
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        Option::<u64>::deserialize(deserializer).map(|ms| ms.map(Duration::from_millis))
    }
}

#[cfg(test)]
mod tests;
