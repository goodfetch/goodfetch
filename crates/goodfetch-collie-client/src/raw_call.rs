//! The raw catalog call (ADR 0099, G8): `call(operation)` on the
//! Connection and on every handle, the way down to an operation the SDK
//! does not generate, such as one a newer engine added, or one of
//! Foxhound's from a Collie program on a Foxhound engine. Its params and
//! its result are the wire's JSON, in snake case, as `serde_json` values.
//! On a value, the operation must be in the value's namespace, and the
//! value's ID fills its parameter. The rest is the main path's: a closed
//! value fails with `closed`, the timeout applies, failures carry the same
//! codes, nothing is retried, and the engine validates the params.

use std::future::IntoFuture;
use std::time::Duration;

use goodfetch_protocol::jsonrpc::{META, RequestMeta};
use serde_json::{Map, Value};

use crate::call::{Call, Note, Reply, Timeout, WIRE_TIMEOUT};
use crate::error::Error;
use crate::rpc::Sender;

/// What a handle's raw call checks when it is awaited: whether its value
/// is closed.
type Closed = Box<dyn FnOnce() -> bool + Send + 'static>;

/// The `_meta` of a raw call: its raw mark (ADR 0099), which tells the
/// engine the program made it through the raw call.
fn raw_mark() -> Value {
    serde_json::to_value(RequestMeta {
        raw: true,
        ..RequestMeta::default()
    })
    .expect("invariant: a request's `_meta` serializes")
}

/// The value a handle's raw call acts on.
pub(crate) struct RawValue {
    /// Its namespace, such as `page`.
    pub(crate) namespace: &'static str,
    /// The parameter its ID fills, such as `page_id`.
    pub(crate) param: &'static str,
    /// Its ID as the wire's JSON, or why it does not serialize.
    pub(crate) id: Result<Value, Error>,
    /// Whether it is closed, read when the call is awaited.
    pub(crate) closed: Closed,
    /// What its handle notes of the reply, if anything.
    pub(crate) note: Option<Note>,
}

/// A raw call of an operation, its params and its result the wire's JSON
/// (ADR 0099), marked raw in its `_meta`. Nothing is sent until it is
/// awaited; then an operation of another namespace than the value's, or
/// params that are no object, hold a `_meta`, name the value's own
/// parameter or give `timeout` beside the option, fail with
/// `invalid_params` before anything is sent, as does a call on a closed
/// value with `closed`. It is never retried.
#[must_use = "a call is sent only when it is awaited"]
pub struct RawCall {
    sender: Sender,
    operation: String,
    params: Value,
    /// The operation's own `timeout`, sent only when the program gave it
    /// (ADR 0080).
    timeout: Timeout,
    value: Option<RawValue>,
}

impl RawCall {
    /// The raw call of `operation` over `sender`, on `value` when a handle
    /// makes it.
    pub(crate) fn new(sender: Sender, operation: &str, value: Option<RawValue>) -> Self {
        Self {
            sender,
            operation: operation.to_owned(),
            params: Value::Object(Map::new()),
            timeout: Timeout::Default,
            value,
        }
    }

    /// Sends `params`, a JSON object in the wire's snake case, such as
    /// `json!({"blocked": true})`; none sends `{}`. A parameter naming a
    /// second value takes that value's ID, as a Run's `id()`.
    pub fn params(mut self, params: Value) -> Self {
        self.params = params;
        self
    }

    /// Sends `timeout` as the operation's own `timeout` parameter (ADR
    /// 0080), in milliseconds, rounded up: the engine fails the call with
    /// `deadline_exceeded` past it. Without it, the operation's setting
    /// decides, or a `timeout` in the params.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Timeout::After(timeout);
        self
    }

    /// Sends the operation's own `timeout` as `null`: it waits as long as
    /// it takes.
    pub fn no_timeout(mut self) -> Self {
        self.timeout = Timeout::Off;
        self
    }

    /// The call the main path sends, or why it is refused before it is.
    fn checked(self) -> Result<Call<Value, Value>, Error> {
        let Self {
            sender,
            operation,
            params,
            timeout,
            value,
        } = self;
        let Value::Object(mut params) = params else {
            return Err(Error::refused(
                &operation,
                "its params are no JSON object",
                "Give its params as an object, such as `json!({\"name\": value})`.",
            ));
        };
        if params.contains_key(META) {
            return Err(Error::refused(
                &operation,
                &format!("`{META}` is the call's envelope, which the SDK writes"),
                &format!("Leave `{META}` out of its params."),
            ));
        }
        if let Some(value) = &value {
            if operation.split('.').next() != Some(value.namespace) {
                return Err(Error::refused(
                    &operation,
                    &format!(
                        "the value it is called on takes only operations of `{}`",
                        value.namespace
                    ),
                    "Call it on the value of its namespace, or on the Connection when it acts on no value.",
                ));
            }
            if params.contains_key(value.param) {
                return Err(Error::refused(
                    &operation,
                    &format!("`{}` is the value's own, which the call fills", value.param),
                    &format!(
                        "Leave `{}` out, or call the operation on the value it names.",
                        value.param
                    ),
                ));
            }
        }
        if timeout != Timeout::Default && params.contains_key(WIRE_TIMEOUT) {
            return Err(Error::refused(
                &operation,
                "`timeout` is given both in its params and as its option",
                "Give the timeout once, as the option.",
            ));
        }
        let mut note = None;
        if let Some(value) = value {
            if (value.closed)() {
                return Err(Error::closed().in_call(&operation));
            }
            params.insert(value.param.to_owned(), value.id?);
            note = value.note;
        }
        params.insert(META.to_owned(), raw_mark());
        let call = Call::new(sender, &operation, Ok(Value::Object(params)));
        let call = match timeout {
            Timeout::Default => call,
            Timeout::Off => call.no_timeout().wire_timeout(),
            Timeout::After(after) => call.timeout(after).wire_timeout(),
        };
        Ok(match note {
            Some(note) => call.note(note),
            None => call,
        })
    }
}

impl IntoFuture for RawCall {
    type Output = Result<Value, Error>;
    type IntoFuture = Reply<Value>;

    fn into_future(self) -> Self::IntoFuture {
        match self.checked() {
            Ok(call) => call.into_future(),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
}

/// C-SEND-SYNC: a raw call moves between threads and outlives the borrow
/// that made it.
const _: () = {
    const fn send_static<T: Send + 'static>() {}
    send_static::<RawCall>();
};
