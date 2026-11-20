//! What dispatch takes of a request's params before its handler runs: the
//! Trace context their `_meta` holds and the Run ID and Session ID the
//! request's span records (ADRs 0077 and 0132), and the page a page call
//! names, which its guard reads. The read into the handler's type notes
//! them in its own pass ([`Params::read`](super::Params::read)); a request
//! whose params no read went through whole, as one its handler refuses,
//! has them read here, where they are.

use goodfetch_protocol::jsonrpc::{META, ProtocolFailure, RequestMeta, TraceContext};
use serde::Deserializer as _;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

use super::{Outer, Strict, without_position};
use crate::call_error::CallError;
use crate::trace;

/// What dispatch takes of a request's params.
#[derive(Debug, Default)]
pub(crate) struct Preamble<'a> {
    /// The Trace context of their `_meta`.
    pub(crate) meta: Option<TraceContext>,
    /// Their Run ID and Session ID.
    pub(crate) ids: trace::Ids<'a>,
}

/// What a pass through a request's params notes where it is, each the
/// last of its name: what dispatch takes of them and the page they name.
#[derive(Default)]
pub(super) struct Noted<'a> {
    /// What dispatch takes of them.
    pub(super) found: Found<'a>,
    /// The page they name themselves, their `page_id`, not read yet.
    pub(super) page: Option<&'a RawValue>,
    /// The page their locator names, its `page_id`, not read yet.
    pub(super) located: Option<&'a RawValue>,
}

impl<'a> Noted<'a> {
    /// Notes them in `params` in a pass that skips every other member.
    /// Params that are no object hold none, and their handler refuses
    /// them.
    pub(super) fn scan(params: &'a str) -> Self {
        let mut noted = Self::default();
        let mut json = serde_json::Deserializer::from_str(params);
        let scanned = Strict {
            inner: &mut json,
            outer: Outer::Params(&mut noted),
        }
        .deserialize_any(IgnoredAny);
        if scanned.is_ok() {
            noted
        } else {
            Self::default()
        }
    }

    /// The page they name: their own `page_id` when it is a string, else
    /// their locator's.
    pub(super) fn page(&self) -> Option<&'a RawValue> {
        self.page
            .filter(|page| page.get().starts_with('"'))
            .or(self.located)
    }
}

/// The members of a request's params dispatch takes, where they are.
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
    /// What dispatch takes of them: no Trace context when they hold no
    /// `_meta`.
    ///
    /// # Errors
    ///
    /// `invalid_params` when their `_meta` holds a key other than
    /// `traceparent`, `tracestate` and `raw`, or a value of another type.
    pub(super) fn checked(self) -> Result<Preamble<'a>, CallError> {
        let meta = self.meta.map(trace_context).transpose()?;
        Ok(Preamble {
            meta,
            ids: self.ids,
        })
    }
}

/// The Trace context a request's `_meta` holds, once the whole `_meta` is
/// checked, its raw mark (ADR 0099) included.
fn trace_context(meta: &RawValue) -> Result<TraceContext, CallError> {
    let refused = |error: String| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{META}` is an object of `traceparent` and `tracestate`, each a string, and `raw`, a boolean: {error}"
            ),
        )
    };
    // serde would read a struct from an array too, which the schema refuses.
    if !meta.get().starts_with('{') {
        return Err(refused("it is no object".to_owned()));
    }
    serde_json::from_str(meta.get())
        .map(RequestMeta::trace_context)
        .map_err(|error| refused(without_position(&error)))
}
