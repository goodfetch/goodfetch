//! What a reader keeps of a value a newer engine adds to an open
//! enumeration or tagged union (ADR 0079): the value as received, never an
//! error. Each open type's `Unknown` variant holds one, and only a reader
//! makes one, so a known value never reads as unknown: an unknown Verdict
//! is never `verified`. The schema leaves the variant out, and
//! [`known_members`] keeps it as the closed type's, so the other layers'
//! types stay as they are.

use std::fmt;

use schemars::Schema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// A value of an open enumeration that this reader does not know, kept as
/// received.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnknownValue(String);

impl UnknownValue {
    /// The value as received.
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The error a request that names this value is refused with: a
    /// request is not covered by ADR 0079's keeping of a newer engine's
    /// values. `what` names the enumeration, and `known` its values.
    pub(crate) fn refused<E: serde::de::Error>(&self, what: &str, known: &[&str]) -> E {
        E::custom(format_args!(
            "unknown {what} `{self}`, expected one of {}",
            known.join(", ")
        ))
    }
}

impl fmt::Display for UnknownValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The field an open tagged union's members are told apart by.
const TAG: &str = "kind";

/// A member of an open tagged union that this reader does not know, kept as
/// received: its tag, in `kind`, and every other field it was sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct UnknownMember(Map<String, Value>);

impl UnknownMember {
    /// The tag it was sent.
    pub fn tag(&self) -> &str {
        self.0
            .get(TAG)
            .and_then(Value::as_str)
            .expect("invariant: a member is read only with its tag")
    }

    /// Its tag as a value of its union's open kind enumeration, which the
    /// tag names.
    pub(crate) fn tag_value(&self) -> UnknownValue {
        UnknownValue(self.tag().to_owned())
    }

    /// Its fields as received, its tag included.
    pub const fn fields(&self) -> &Map<String, Value> {
        &self.0
    }

    /// Reads the unknown member of an open tagged union whose known members
    /// carry the tags `known`. Serde falls back to the unknown member
    /// whenever a known member does not read, so a member with a known tag
    /// is refused here: a known member whose fields are wrong is an error,
    /// never an unknown member.
    pub(crate) fn read_unknown<'de, D: Deserializer<'de>>(
        deserializer: D,
        known: &[&str],
    ) -> Result<Self, D::Error> {
        let member = Self::deserialize(deserializer)?;
        if known.contains(&member.tag()) {
            return Err(D::Error::custom(format_args!(
                "the member `{}` is known, and its fields do not read",
                member.tag()
            )));
        }
        Ok(member)
    }
}

impl<'de> Deserialize<'de> for UnknownMember {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = Map::deserialize(deserializer)?;
        match fields.get(TAG) {
            Some(Value::String(_)) => Ok(Self(fields)),
            _ => Err(D::Error::custom(format_args!(
                "a member of an open tagged union names itself by a string `{TAG}`"
            ))),
        }
    }
}

/// An open type's schema as its known values alone give it: `schemars`
/// writes `anyOf` for an enum with an untagged variant, its skipped
/// `Unknown` included, where the known values, each told apart by its own
/// constant, are a `oneOf`.
pub(crate) fn known_members(schema: &mut Schema) {
    if let Some(members) = schema.remove("anyOf") {
        schema.insert("oneOf".to_owned(), members);
    }
}

#[cfg(test)]
mod tests;
