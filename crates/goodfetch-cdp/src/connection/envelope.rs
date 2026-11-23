//! What the driver reads of a browser's message to route it: its envelope,
//! as JSON text where it is in the message, and nothing else.

use std::fmt;
use std::ops::Range;

use serde::de::{self, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

/// The members of a browser's message that route it, each the last of its
/// name, as JSON text where it is in the message: nothing else in it is
/// decoded, and these only as far as routing needs.
#[derive(Clone, Copy, Default)]
pub(super) struct Envelope<'a> {
    pub(super) id: Option<&'a RawValue>,
    pub(super) method: Option<&'a RawValue>,
    pub(super) session: Option<&'a RawValue>,
    pub(super) params: Option<&'a RawValue>,
    pub(super) result: Option<&'a RawValue>,
    pub(super) error: Option<&'a RawValue>,
}

impl<'de> Deserialize<'de> for Envelope<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(EnvelopeVisitor)
    }
}

struct EnvelopeVisitor;

impl<'de> Visitor<'de> for EnvelopeVisitor {
    type Value = Envelope<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut members: A) -> Result<Envelope<'de>, A::Error> {
        let mut envelope = Envelope::default();
        while let Some(member) = members.next_key::<Member>()? {
            let slot = match member {
                Member::Id => &mut envelope.id,
                Member::Method => &mut envelope.method,
                Member::Session => &mut envelope.session,
                Member::Params => &mut envelope.params,
                Member::Result => &mut envelope.result,
                Member::Error => &mut envelope.error,
                Member::Other => {
                    members.next_value::<IgnoredAny>()?;
                    continue;
                }
            };
            *slot = Some(members.next_value()?);
        }
        Ok(envelope)
    }
}

/// The name of a member of the envelope, escaped or not.
enum Member {
    Id,
    Method,
    Session,
    Params,
    Result,
    Error,
    Other,
}

impl<'de> Deserialize<'de> for Member {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Name;
        impl Visitor<'_> for Name {
            type Value = Member;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a member's name")
            }

            fn visit_str<E: de::Error>(self, name: &str) -> Result<Member, E> {
                Ok(match name {
                    "id" => Member::Id,
                    "method" => Member::Method,
                    "sessionId" => Member::Session,
                    "params" => Member::Params,
                    "result" => Member::Result,
                    "error" => Member::Error,
                    _ => Member::Other,
                })
            }
        }
        deserializer.deserialize_identifier(Name)
    }
}

/// A member's string, when it is one.
pub(super) fn text(member: &RawValue) -> Option<String> {
    serde_json::from_str(member.get()).ok()
}

/// Where `part`, a slice of `message`, is in it.
pub(super) fn range_in(message: &[u8], part: &str) -> Range<usize> {
    let start = (part.as_ptr() as usize) - (message.as_ptr() as usize);
    start..start + part.len()
}
