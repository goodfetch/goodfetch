//! The builders every call goes through (spec #606, ADR 0099): a [`Call`]
//! waits for its reply and takes `.timeout(d)` or `.no_timeout()`; a
//! [`Query`] asks what the engine answers at once and takes no timeout.
//! Each holds its typed params, `P`, until it is sent, so the members the
//! catalog gives a handle set their options on them; awaiting either only
//! exchanges messages with the Connection's runtime, so it runs on any
//! executor (ADR 0081). Dropping the future once its call is sent cancels
//! the call with `$/cancel`, without waiting, when the catalog says the
//! call is cancellable (ADR 0080).

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::entries;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::error::Error;
use crate::rpc::{CallId, Outgoing, Sender};
use crate::trace_context::{self, TraceContext};

/// A call's reply, as `.await` gives it.
pub type Reply<T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'static>>;

/// How a call reads its reply's JSON: as its result type, then through
/// what [`Call::map`] adds.
type Read<T> = Box<dyn FnOnce(Value) -> Result<T, Error> + Send + 'static>;

/// What the handle a call came from notes of its reply's JSON before the
/// caller reads it: a Run notes that it ended.
pub(crate) type Note = Box<dyn FnOnce(&Value) + Send + 'static>;

/// What answers a call before it is sent, so that it is not: a close of a
/// value already closed answers at once.
type Settled = Box<dyn FnOnce() -> Option<Value> + Send + 'static>;

/// The name of the params member an operation's own `timeout` travels in
/// (ADR 0080).
pub(crate) const WIRE_TIMEOUT: &str = "timeout";

/// How long a call waits for its reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Timeout {
    /// The engine's default for an operation that takes its own `timeout`;
    /// for any other, as long as the Connection lasts, until the
    /// Connection takes a scope's default (spec #606).
    Default,
    /// As long as the Connection lasts.
    Off,
    /// At most this long.
    After(Duration),
}

/// A call that waits for its reply, `T`, with its params, `P`. Nothing is
/// sent until it is awaited, and dropping the future once a cancellable
/// call is sent cancels it with `$/cancel`, without waiting (ADR 0080).
#[must_use = "a call is sent only when it is awaited"]
pub struct Call<T, P = Value> {
    request: Request<P>,
    timeout: Timeout,
    /// Whether the operation takes its own `timeout`, which the call's
    /// then sets instead of a timer of the client's (ADR 0080).
    wire: bool,
    read: Read<T>,
}

impl<T: DeserializeOwned, P> Call<T, P> {
    /// The call of `method` with `params` over `sender`, its reply read
    /// as `T`.
    pub(crate) fn new(sender: Sender, method: &str, params: Result<P, Error>) -> Self {
        Self {
            request: Request::new(sender, method, params),
            timeout: Timeout::Default,
            wire: false,
            read: read(),
        }
    }
}

impl<T, P> Call<T, P> {
    /// Fails the call if its reply has not come within `timeout`: with
    /// `deadline_exceeded`, or with the engine's `deadline_exceeded` for an
    /// operation that takes its own `timeout`. A reply that comes later
    /// changes nothing.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Timeout::After(timeout);
        self
    }

    /// Waits for the reply as long as the Connection lasts.
    pub fn no_timeout(mut self) -> Self {
        self.timeout = Timeout::Off;
        self
    }

    /// Sends the call's timeout as the operation's own `timeout` member
    /// rather than keeping a timer of the client's (ADR 0080).
    pub(crate) fn wire_timeout(mut self) -> Self {
        self.wire = true;
        self
    }

    /// Changes the params before they are sent: an option's builder
    /// method.
    pub(crate) fn edit(mut self, edit: impl FnOnce(&mut P)) -> Self {
        if let Ok(params) = &mut self.request.params {
            edit(params);
        }
        self
    }

    /// Changes the params before they are sent, or fails the call with the
    /// error `edit` returns, which it then answers without sending: an
    /// option's builder method that checks its value, as `settings` does.
    pub(crate) fn try_edit(mut self, edit: impl FnOnce(&mut P) -> Result<(), Error>) -> Self {
        if let Ok(params) = &mut self.request.params
            && let Err(error) = edit(params)
        {
            self.request.params = Err(error);
        }
        self
    }

    /// Runs `note` with the reply's JSON once it has come.
    pub(crate) fn note(mut self, note: impl FnOnce(&Value) + Send + 'static) -> Self {
        self.request.note = Some(Box::new(note));
        self
    }

    /// Runs `settled` when the call is awaited, before anything is sent:
    /// the JSON it returns is the call's reply, and the call is not sent.
    pub(crate) fn settled(
        mut self,
        settled: impl FnOnce() -> Option<Value> + Send + 'static,
    ) -> Self {
        self.request.settled = Some(Box::new(settled));
        self
    }

    /// The call whose reply is `map` of this one's: an opener's handle, or
    /// the one property of a result (issue 2517).
    pub(crate) fn map<U>(self, map: impl FnOnce(T) -> U + Send + 'static) -> Call<U, P>
    where
        T: 'static,
    {
        let read = self.read;
        Call {
            request: self.request,
            timeout: self.timeout,
            wire: self.wire,
            read: Box::new(move |value| read(value).map(map)),
        }
    }
}

impl<T: Send + 'static, P: Serialize + Send + 'static> IntoFuture for Call<T, P> {
    type Output = Result<T, Error>;
    type IntoFuture = Reply<T>;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            request,
            timeout,
            wire,
            read,
        } = self;
        let timer = match timeout {
            Timeout::After(after) if !wire => Some(after),
            _ => None,
        };
        let traced = request.sender.trace_context();
        Box::pin(async move {
            let reply = request
                .send(traced, timer, |params| {
                    if wire {
                        set_wire_timeout(params, timeout);
                    }
                })
                .await?;
            read(reply)
        })
    }
}

/// A question the engine answers at once, `T`, with its params, `P`,
/// without waiting on a page, a network or anything else, so it takes no
/// timeout. Nothing is sent until it is awaited, and dropping the future
/// once a cancellable query is sent cancels it with `$/cancel`, without
/// waiting.
#[must_use = "a query is sent only when it is awaited"]
pub struct Query<T, P = Value> {
    request: Request<P>,
    read: Read<T>,
}

impl<T: DeserializeOwned, P> Query<T, P> {
    /// The query of `method` with `params` over `sender`, its answer read
    /// as `T`.
    pub(crate) fn new(sender: Sender, method: &str, params: Result<P, Error>) -> Self {
        Self {
            request: Request::new(sender, method, params),
            read: read(),
        }
    }
}

impl<T, P> Query<T, P> {
    /// Changes the params before they are sent: an option's builder
    /// method, as `config.explain`'s `keys`.
    pub(crate) fn edit(mut self, edit: impl FnOnce(&mut P)) -> Self {
        if let Ok(params) = &mut self.request.params {
            edit(params);
        }
        self
    }

    /// The query whose answer is `map` of this one's: the one property of
    /// a result (issue 2517).
    #[expect(
        dead_code,
        reason = "no query of the catalog answers a result of one property yet; `xtask codegen` writes its member through this one"
    )]
    pub(crate) fn map<U>(self, map: impl FnOnce(T) -> U + Send + 'static) -> Query<U, P>
    where
        T: 'static,
    {
        let read = self.read;
        Query {
            request: self.request,
            read: Box::new(move |value| read(value).map(map)),
        }
    }
}

impl<T: 'static, P> Query<T, P> {
    /// The query whose answer is `map` of this one's, or the error `map`
    /// fails with: a read whose values a program's type reads.
    pub(crate) fn try_map<U>(
        self,
        map: impl FnOnce(T) -> Result<U, Error> + Send + 'static,
    ) -> Query<U, P> {
        let read = self.read;
        Query {
            request: self.request,
            read: Box::new(move |value| read(value).and_then(map)),
        }
    }
}

impl<T: Send + 'static, P: Serialize + Send + 'static> IntoFuture for Query<T, P> {
    type Output = Result<T, Error>;
    type IntoFuture = Reply<T>;

    fn into_future(self) -> Self::IntoFuture {
        let Self { request, read } = self;
        let traced = request.sender.trace_context();
        Box::pin(async move { read(request.send(traced, None, |_| ()).await?) })
    }
}

/// What a call or a query sends.
struct Request<P> {
    sender: Sender,
    method: String,
    params: Result<P, Error>,
    note: Option<Note>,
    settled: Option<Settled>,
}

impl<P> Request<P> {
    fn new(sender: Sender, method: &str, params: Result<P, Error>) -> Self {
        Self {
            sender,
            method: method.to_owned(),
            params,
            note: None,
            settled: None,
        }
    }
}

impl<P: Serialize> Request<P> {
    /// Sends the request, its params as JSON once `edit` changed them,
    /// carrying `traced`, the Trace context its caller's thread had, and
    /// answers its reply's JSON once the handle noted it; a timer of the
    /// client's fails it past `timeout`. A request its handle settles
    /// first is not sent, and a cancellable one dropped before its reply is
    /// cancelled.
    async fn send(
        self,
        traced: Option<TraceContext>,
        timeout: Option<Duration>,
        edit: impl FnOnce(&mut Value),
    ) -> Result<Value, Error> {
        self.sender
            .ensure_opener()
            .map_err(|forked| forked.in_call(&self.method))?;
        if let Some(answer) = self.settled.and_then(|settled| settled()) {
            return Ok(answer);
        }
        // A value closed before the call fails it headed by the call, as the
        // Connection's own end does (ADR 0079's note of 2026-11-19).
        let params = self.params.map_err(|error| error.in_call(&self.method))?;
        let mut params = serde_json::to_value(params).map_err(Error::shape)?;
        edit(&mut params);
        if let Some(traced) = traced {
            trace_context::carry(&mut params, traced);
        }
        let (reply, replied) = oneshot::channel();
        let call = Outgoing {
            method: self.method,
            params,
            timeout,
            reply,
            notified: None,
        };
        let mut waiting = if cancellable(&call.method) {
            let call = self.sender.call_cancellable(call);
            Some(Waiting {
                sender: self.sender,
                call,
                replied: false,
            })
        } else {
            self.sender.call(call);
            None
        };
        let replied = replied.await;
        if let Some(waiting) = &mut waiting {
            waiting.replied = true;
        }
        let result = replied.map_err(|_| Error::closed())??;
        if let Some(note) = self.note {
            note(&result);
        }
        Ok(result)
    }
}

/// Whether the catalog says a Client may cancel a call of `method` in
/// flight; the engine ignores `$/cancel` for any other call, including one
/// of a method the catalog does not know.
fn cancellable(method: &str) -> bool {
    entries::operations_of(Product::Foxhound)
        .any(|operation| operation.name == method && operation.cancellable)
}

/// A sent cancellable call its caller waits on: dropped before its reply, as when the
/// caller drops the call's future, it cancels the call with `$/cancel`
/// without waiting (ADR 0080).
struct Waiting {
    sender: Sender,
    call: CallId,
    replied: bool,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if !self.replied {
            self.sender.cancel_sent(&self.call);
        }
    }
}

/// Reads a reply's JSON as `T`.
fn read<T: DeserializeOwned>() -> Read<T> {
    Box::new(|value| serde_json::from_value(value).map_err(Error::shape))
}

/// Sets the operation's own `timeout` in `params` (ADR 0080): the engine's
/// default leaves it out, no timeout sends it null, and a duration sends
/// it in milliseconds, rounded up.
fn set_wire_timeout(params: &mut Value, timeout: Timeout) {
    let Value::Object(members) = params else {
        return;
    };
    match timeout {
        Timeout::Default => {
            members.remove(WIRE_TIMEOUT);
        }
        Timeout::Off => {
            members.insert(WIRE_TIMEOUT.to_owned(), Value::Null);
        }
        Timeout::After(after) => {
            members.insert(WIRE_TIMEOUT.to_owned(), Value::from(wire_millis(after)));
        }
    }
}

/// `after` in whole milliseconds, rounded up, as a timeout is sent and
/// reported.
pub(crate) fn wire_millis(after: Duration) -> u64 {
    u64::try_from(after.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX)
}

/// C-SEND-SYNC: a call and a query, and the futures they become, move
/// between threads and outlive the borrow that made them.
const _: () = {
    const fn send_static<T: Send + 'static>() {}
    send_static::<Call<Value>>();
    send_static::<Query<Value>>();
    send_static::<Reply<Value>>();
};
