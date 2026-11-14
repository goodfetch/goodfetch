//! What dispatch takes of a request's params before its handler runs: the
//! Trace context their `_meta` holds and the Run ID and Session ID the
//! request's span records (ADRs 0077 and 0132). The read into the handler's
//! type notes them in its own pass ([`Params::read`](super::Params::read));
//! a request whose params no read went through whole, as one its handler
//! refuses, has them read here, where they are.

use std::fmt;

use goodfetch_protocol::jsonrpc::{META, ProtocolFailure, TraceContext};
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

use super::without_position;
use crate::call_error::CallError;
use crate::trace::{self, ID_FIELDS};
use crate::wire::{Key, KeySeed, text};

/// What dispatch takes of a request's params.
#[derive(Debug, Default)]
pub(crate) struct Preamble<'a> {
    /// The Trace context of their `_meta`.
    pub(crate) meta: Option<TraceContext>,
    /// Their Run ID and Session ID.
    pub(crate) ids: trace::Ids<'a>,
}

/// The members of a request's params dispatch takes, where they are, each
/// the last of its name.
#[derive(Default)]
pub(super) struct Found<'a> {
    /// Their `_meta`, not read yet.
    pub(super) meta: Option<&'a RawValue>,
    /// Their Run ID and Session ID.
    pub(super) ids: trace::Ids<'a>,
    /// Whether the pass that found them went through every member of the
    /// params to their end: a read that skips them finds nothing.
    pub(super) whole: bool,
}

impl<'a> Found<'a> {
    /// Reads them from `params` where they are. Params that are no object
    /// hold none, and their handler refuses them.
    pub(super) fn scan(params: &'a str) -> Self {
        serde_json::from_str(params).unwrap_or_default()
    }

    /// What dispatch takes of them: no Trace context when they hold no
    /// `_meta`.
    ///
    /// # Errors
    ///
    /// `invalid_params` when their `_meta` holds a key other than
    /// `traceparent` and `tracestate` or a value that is no string.
    pub(super) fn checked(self) -> Result<Preamble<'a>, CallError> {
        let meta = self.meta.map(trace_context).transpose()?;
        Ok(Preamble {
            meta,
            ids: self.ids,
        })
    }
}

impl<'de> Deserialize<'de> for Found<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(FoundVisitor)
    }
}

struct FoundVisitor;

impl<'de> Visitor<'de> for FoundVisitor {
    type Value = Found<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut members: A) -> Result<Self::Value, A::Error> {
        let mut found = Found::default();
        while let Some(Key(key)) = members.next_key_seed(KeySeed)? {
            if key == META {
                found.meta = Some(members.next_value()?);
            } else if let Some(at) = ID_FIELDS.iter().position(|field| *field == key) {
                let value: &RawValue = members.next_value()?;
                found.ids[at] = text(value);
            } else {
                members.next_value::<IgnoredAny>()?;
            }
        }
        Ok(found)
    }
}

/// The Trace context a request's `_meta` holds.
fn trace_context(meta: &RawValue) -> Result<TraceContext, CallError> {
    let refused = |error: String| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{META}` is an object of `traceparent` and `tracestate`, each a string: {error}"
            ),
        )
    };
    // serde would read a struct from an array too, which the schema refuses.
    if !meta.get().starts_with('{') {
        return Err(refused("it is no object".to_owned()));
    }
    serde_json::from_str(meta.get()).map_err(|error| refused(without_position(&error)))
}
