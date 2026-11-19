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

use serde_json::{Map, Value};

use crate::call::{Call, Reply};
use crate::error::Error;
use crate::rpc::Sender;

/// The name of the params member an operation's own `timeout` travels in
/// (ADR 0080).
const WIRE_TIMEOUT: &str = "timeout";

/// What a handle's raw call checks when it is awaited: whether its value
/// is closed.
type Closed = Box<dyn FnOnce() -> bool + Send + 'static>;

/// What a Run's handle notes of a reply: that its Run ended.
pub(crate) type RawNote = Box<dyn FnOnce(&Value) + Send + 'static>;

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
    pub(crate) note: Option<RawNote>,
}

/// The operation's own `timeout` the program gave (ADR 0080).
#[derive(Clone, Copy, Debug)]
enum Timeout {
    /// None: sent as `null`.
    Off,
    /// At most this long, sent in milliseconds, rounded up.
    After(Duration),
}

/// A raw call of an operation, its params and its result the wire's JSON
/// (ADR 0099). Nothing is sent until it is awaited; then an operation of
/// another namespace than the value's, or params naming the value's own
/// parameter, fail with `invalid_params` before anything is sent, as does
/// a call on a closed value with `closed`. It is never retried.
#[must_use = "a call is sent only when it is awaited"]
pub struct RawCall {
    sender: Sender,
    operation: String,
    params: Value,
    timeout: Option<Timeout>,
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
            timeout: None,
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
        self.timeout = Some(Timeout::After(timeout));
        self
    }

    /// Sends the operation's own `timeout` as `null`: it waits as long as
    /// it takes.
    pub fn no_timeout(mut self) -> Self {
        self.timeout = Some(Timeout::Off);
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
        if timeout.is_some() && params.contains_key(WIRE_TIMEOUT) {
            return Err(Error::refused(
                &operation,
                "`timeout` is given both in its params and as its option",
                "Give the timeout once, as the option.",
            ));
        }
        let mut note = None;
        if let Some(value) = value {
            if (value.closed)() {
                return Err(Error::closed());
            }
            params.insert(value.param.to_owned(), value.id?);
            note = value.note;
        }
        let call = Call::new(sender, &operation, Ok(Value::Object(params)));
        let call = match timeout {
            None => call,
            Some(Timeout::Off) => call.no_timeout().wire_timeout(),
            Some(Timeout::After(after)) => call.timeout(after).wire_timeout(),
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
