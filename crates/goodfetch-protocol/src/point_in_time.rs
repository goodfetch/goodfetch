//! Points in time on the wire (ADR 0092; spec 606, Casing and times): every
//! field the schema declares as a `date-time` is a [`SystemTime`] in Rust,
//! written as RFC 3339 in UTC and read from RFC 3339 with any offset, which
//! is converted to UTC. A field takes `#[serde(with = "…::point_in_time")]`
//! (`point_in_time::nullable` for an `Option`) and, for its schema,
//! `#[schemars(with = "PointInTime")]`.

use std::borrow::Cow;
use std::time::SystemTime;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serializer};

/// The schema of a point in time, the `$defs` entry each `date-time` field
/// refers to; the value itself is a [`SystemTime`].
pub enum PointInTime {}

impl JsonSchema for PointInTime {
    fn schema_name() -> Cow<'static, str> {
        "PointInTime".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A point in time in UTC, as RFC 3339.",
            "type": "string",
            "format": "date-time",
        })
    }
}

/// The RFC 3339 text of `at` in UTC, as the wire carries it, with as many
/// fractional digits as it needs.
///
/// # Errors
///
/// When `at` is outside the years 9999 BC to 9999 AD, which RFC 3339 cannot
/// write.
pub fn to_rfc3339(at: SystemTime) -> Result<String, jiff::Error> {
    jiff::Timestamp::try_from(at).map(|at| at.to_string())
}

/// The point in time RFC 3339 `text` names, whatever its offset.
///
/// # Errors
///
/// When `text` is no RFC 3339 point in time with an offset.
pub fn from_rfc3339(text: &str) -> Result<SystemTime, jiff::Error> {
    text.parse::<jiff::Timestamp>().map(SystemTime::from)
}

/// Writes `at` as [`to_rfc3339`] does.
///
/// # Errors
///
/// As [`to_rfc3339`], or the serializer's.
pub fn serialize<S: Serializer>(at: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
    let text = to_rfc3339(*at).map_err(serde::ser::Error::custom)?;
    serializer.serialize_str(&text)
}

/// Reads a point in time as [`from_rfc3339`] does.
///
/// # Errors
///
/// As [`from_rfc3339`], or the deserializer's.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
    let text = Cow::<str>::deserialize(deserializer)?;
    from_rfc3339(&text).map_err(serde::de::Error::custom)
}

/// A nullable point in time: `null` is `None`. A field that may be left out
/// takes `#[serde(default)]` too.
pub mod nullable {
    use std::borrow::Cow;
    use std::time::SystemTime;

    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes `at`, or `null`.
    ///
    /// # Errors
    ///
    /// As [`super::serialize`].
    pub fn serialize<S: Serializer>(
        at: &Option<SystemTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match at {
            Some(at) => super::serialize(at, serializer),
            None => serializer.serialize_none(),
        }
    }

    /// Reads a point in time, or `null` as `None`.
    ///
    /// # Errors
    ///
    /// As [`super::deserialize`].
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        Option::<Cow<'de, str>>::deserialize(deserializer)?
            .map(|text| super::from_rfc3339(&text).map_err(serde::de::Error::custom))
            .transpose()
    }
}

#[cfg(test)]
mod tests;
