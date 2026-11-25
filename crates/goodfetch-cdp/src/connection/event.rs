//! An event as the browser sent it, and the one decode of its params that
//! all its readers share (ADR 0140's `C1.event_decodes`).

use std::any::Any;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use serde_json::Value;

use super::SessionId;

/// An event as the browser sent it: its method, which routed it, and the
/// browser's message, which it keeps whole, so its params are decoded from
/// the message's bytes once, by the first reader that reads them, and every
/// other reader of the event takes a copy of that decode.
#[derive(Clone, Debug)]
pub struct Event {
    /// `Domain.event`.
    pub method: String,
    /// The target's session it came on; none for the browser's own.
    session: Option<SessionId>,
    /// Its number among the events the connection took in, from 1: a
    /// subscription takes those numbered after it started.
    number: u64,
    /// The browser's message.
    message: Bytes,
    /// Where its params are in `message`; none when it has none.
    params: Option<Range<usize>>,
    /// Its params as the first reader decoded them; none when they did not
    /// read as that reader's.
    decoded: OnceLock<Option<Arc<dyn Any + Send + Sync>>>,
    /// Where each decode of its params is counted (ADR 0140's
    /// `C1.event_decodes`).
    work: CountedWork,
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.method == other.method && self.params() == other.params()
    }
}

impl Event {
    /// The event `method` with `params`, as a browser sends one on its own
    /// session, numbered 0, as no connection took it in; its decodes are
    /// counted on a work of its own.
    pub fn new(method: impl Into<String>, params: &Value) -> Self {
        let message =
            Bytes::from(serde_json::to_vec(params).expect("invariant: a JSON value serializes"));
        let params = Some(0..message.len());
        Self::sent(
            (method.into(), None, 0),
            message,
            params,
            CountedWork::default(),
        )
    }

    /// The event `method` the browser sent on `session` as `message`, the
    /// `number`th the connection took in, its params at `params` in it; its
    /// decodes are counted on `work`.
    pub(super) fn sent(
        (method, session, number): (String, Option<SessionId>, u64),
        message: Bytes,
        params: Option<Range<usize>>,
        work: CountedWork,
    ) -> Self {
        Self {
            method,
            session,
            number,
            message,
            params,
            decoded: OnceLock::new(),
            work,
        }
    }

    /// The target's session it came on; `None` for the browser's own.
    pub fn session(&self) -> Option<&SessionId> {
        self.session.as_ref()
    }

    /// Its number among the events the connection took in.
    pub(super) fn number(&self) -> u64 {
        self.number
    }

    /// Its params' JSON text: `null` when it has none.
    fn params(&self) -> &[u8] {
        self.params
            .clone()
            .map_or(b"null".as_slice(), |params| &self.message[params])
    }

    /// The bytes it holds, which count against its readers' quota.
    pub(super) fn bytes(&self) -> usize {
        self.message.len()
    }

    /// Its params as `E`'s, when it is `E`, decoded from the browser's
    /// message by the first reader and copied for every other; params that
    /// do not read as `E`'s, which only a browser the bindings do not match
    /// sends, are logged at `warn` once, with the method, never with the
    /// params.
    pub fn read<E: crate::Event>(&self) -> Option<E> {
        if self.method != E::METHOD {
            return None;
        }
        let decoded = self.decoded.get_or_init(|| {
            self.decode::<E>()
                .map(|params| Arc::new(params) as Arc<dyn Any + Send + Sync>)
        });
        match decoded {
            None => None,
            // Two event types of one method: the second decodes its own.
            Some(decoded) => decoded
                .downcast_ref::<E>()
                .cloned()
                .or_else(|| self.decode::<E>()),
        }
    }

    /// Its params decoded as `E`'s, counted.
    fn decode<E: crate::Event>(&self) -> Option<E> {
        self.work.count(Row::CdpEventDecodes);
        serde_json::from_slice::<E>(self.params())
            .inspect_err(|_| {
                tracing::warn!(
                    cdp.method = E::METHOD,
                    "the browser sent an event whose params do not read as the bindings say"
                );
            })
            .ok()
    }
}
