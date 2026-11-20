//! Why a Connection or one of its calls failed (ADR 0079): one opaque
//! [`Error`] whose code is the open [`ErrorCode`] (ADR 0099), with an
//! accessor per field the other SDKs' errors carry and its code's details
//! read by type. And [`BoxError`], the one error a program's own code
//! returns, read as ADR 0109 says.

use std::fmt;
use std::io;
use std::path::Path;

use goodfetch_protocol::catalog::{ErrorCodeDefinition, ErrorGroup};
use goodfetch_protocol::checkpoint::{
    FILE_WRITE_FAILED, FileWriteFailedDetails, PATH_NOT_WRITABLE, PathNotWritableDetails,
};
use goodfetch_protocol::connection::{
    CLOSED, CONNECTION_LOST, DEADLINE_EXCEEDED, ENGINE_EXITED, Form, INTERNAL,
};
use goodfetch_protocol::daemon::DAEMON_UNREACHABLE;
use goodfetch_protocol::entries;
use goodfetch_protocol::error_code::{ErrorCode, ErrorDetails};
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject, ProtocolFailure};
use goodfetch_protocol::naming;
use goodfetch_protocol::page::RESPONSE_NOT_JSON;
use goodfetch_protocol::page::events::BODY_UNAVAILABLE;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingLimit};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The error a program's own code returns, from a handler, a Hook, a login
/// step or any other function the SDK calls (ADR 0109): std's
/// `Box<dyn Error + Send + Sync>`, which `?` converts any such error into,
/// a `&str`, a `String` and the SDK's own [`Error`] included.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// The type name the details of a program's own error carry from Rust
/// (ADR 0109): the box erases the concrete type.
const BOX_ERROR: &str = "BoxError";

/// Why a Connection or one of its calls failed: an Error code, its group,
/// whether a retry may succeed, the message, the fields every error
/// carries and the code's details (ADR 0079). Match on [`Error::code`]
/// against [`ErrorCode`]'s constants; its `Display` is its message.
pub struct Error(Box<Inner>);

struct Inner {
    code: ErrorCode,
    group: ErrorGroup,
    retryable: bool,
    message: String,
    /// The fields every error carries and the code's details, in wire
    /// names, as the engine's `data` holds them beside the code.
    data: Map<String, Value>,
    /// Whether the engine answered the call with it.
    answered: bool,
    source: Option<BoxError>,
}

impl Error {
    /// The Error code, as the engine sent it; one this SDK does not know
    /// stays a value.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.0.code
    }

    /// The code's group, named for its remedy.
    #[must_use]
    pub fn group(&self) -> ErrorGroup {
        self.0.group
    }

    /// Whether the same call may succeed when made again.
    #[must_use]
    pub fn retryable(&self) -> bool {
        self.0.retryable
    }

    /// What went wrong, for people, as every SDK shows it; never match on
    /// it.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// The code's details as `D`, its details type, or none when `D` holds
    /// another code's details or the engine's do not fit it.
    #[must_use]
    pub fn details<D: ErrorDetails>(&self) -> Option<D> {
        if !D::CODES.contains(&self.0.code) {
            return None;
        }
        serde_json::from_value(Value::Object(self.0.data.clone())).ok()
    }

    /// What to do about it, when the engine or the SDK knows.
    #[must_use]
    pub fn fix(&self) -> Option<&str> {
        self.text("fix")
    }

    /// The operation the call made, by its canonical catalog name, such as
    /// `locator.click`.
    #[must_use]
    pub fn operation(&self) -> Option<&str> {
        self.text("operation")
    }

    /// The call's ID, which finds it in the Run trace.
    #[must_use]
    pub fn call_id(&self) -> Option<&str> {
        self.text("call_id")
    }

    /// The Run the call belonged to, inside a Run.
    #[must_use]
    pub fn run_id(&self) -> Option<&str> {
        self.text("run_id")
    }

    /// What the call acted on, as the engine rendered it in Rust.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.text("target")
    }

    /// The page's URL, where one applies.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.text("url")
    }

    /// What was there instead of what the call needed.
    #[must_use]
    pub fn observed(&self) -> Option<&str> {
        self.text("observed")
    }

    /// The setting that decided the outcome, with its value, the layer
    /// that set it and where the value came from (ADRs 0079, 0083).
    #[must_use]
    pub fn limit(&self) -> Option<SettingLimit> {
        self.0
            .data
            .get("limit")
            .and_then(|limit| SettingLimit::deserialize(limit).ok())
    }

    /// The engine's steps for the call, as Playwright's call log; empty
    /// when it sent none. Never a Secret's value (ADR 0015).
    #[must_use]
    pub fn call_log(&self) -> Vec<&str> {
        self.0
            .data
            .get("call_log")
            .and_then(Value::as_array)
            .map(|lines| lines.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// The program's own error `error` as the SDK reads it (ADR 0109): a
    /// box that holds the SDK's own [`Error`] keeps it, code and retryable
    /// flag included; anything else, an error that wraps a Goodfetch error
    /// in its `source()` chain included, is the program's own, failing with
    /// `own`, the code the place that called the program gives it, such as
    /// a handler's `handler_failed`. A place that wraps every failure of
    /// the program, a Hook or a `for_each` function, does not call it.
    /// Its details name the type
    /// `BoxError`, its message is the error's `Display` followed by each
    /// `source()`, joined by `": "`, and its stack is empty; the box is its
    /// `source()`.
    #[must_use]
    pub fn of_program(error: BoxError, own: ErrorCode) -> Self {
        match error.downcast::<Self>() {
            Ok(error) => *error,
            Err(error) => {
                let mut chain = error.to_string();
                let mut cause = error.source();
                while let Some(next) = cause {
                    chain.push_str(": ");
                    chain.push_str(&next.to_string());
                    cause = next.source();
                }
                let (group, retryable) = entries::error_code(own.as_str())
                    .map_or((ErrorGroup::Program, false), |defined| {
                        (defined.group, defined.retryable)
                    });
                let mut data = Map::new();
                data.insert("exception_type".to_owned(), BOX_ERROR.into());
                data.insert("exception_message".to_owned(), chain.clone().into());
                data.insert("exception_stack".to_owned(), String::new().into());
                Self(Box::new(Inner {
                    code: own,
                    group,
                    retryable,
                    message: format!("the program's code failed with {BOX_ERROR}: {chain}"),
                    data,
                    answered: false,
                    source: Some(error),
                }))
            }
        }
    }

    /// The linked engine, which runs in the program's own process, could
    /// not start or failed (ADR 0087): `engine_exited`, with `source` as
    /// its cause. A Product crate's linked engine fails with it.
    #[must_use]
    pub fn engine_failed(source: impl Into<BoxError>) -> Self {
        let source = source.into();
        Self::of(
            &ENGINE_EXITED,
            format!("the linked engine failed: {source}"),
        )
        .caused_by(source)
    }

    /// No Daemon could be reached where a Product crate's connection looked
    /// for one: `daemon_unreachable`, with `fix` saying what to do.
    #[must_use]
    pub fn daemon_unreachable(message: String, fix: String) -> Self {
        let mut error = Self::of(&DAEMON_UNREACHABLE, message);
        error.0.data.insert("fix".to_owned(), fix.into());
        error
    }

    /// A value a Product crate's connection reads before it reaches an
    /// engine, such as a remote Daemon's URL, is refused:
    /// `setting_invalid`, with `details`.
    #[must_use]
    pub fn setting_invalid(message: String, details: &SettingInvalidDetails) -> Self {
        let mut error = Self::of(&SETTING_INVALID, message);
        if let Ok(Value::Object(details)) = serde_json::to_value(details) {
            error.0.data.extend(details);
        }
        error
    }

    /// A raw call (ADR 0099) refused before it is sent, as the engine
    /// would refuse its params: `invalid_params`, group `program`, not
    /// retryable, with `fix` and the operation.
    pub(crate) fn refused(operation: &str, reason: &str, fix: &str) -> Self {
        let failure = ProtocolFailure::InvalidParams;
        let mut data = Map::new();
        data.insert("fix".to_owned(), fix.into());
        data.insert("operation".to_owned(), operation.into());
        Self(Box::new(Inner {
            code: ErrorCode::new(failure.name())
                .expect("invariant: a protocol failure's name fits an ErrorCode"),
            group: failure.group(),
            retryable: false,
            message: format!("{operation}: {reason}"),
            data,
            answered: false,
            source: None,
        }))
    }

    /// The error the engine answered a call with.
    pub(crate) fn answered(object: ErrorObject) -> Self {
        let ErrorObject {
            code: class,
            message,
            data,
        } = object;
        let ErrorData {
            code,
            retryable,
            details,
        } = data;
        let group = ErrorGroup::ALL
            .into_iter()
            .find(|group| group.json_rpc_class() == class)
            .or_else(|| {
                ProtocolFailure::ALL
                    .into_iter()
                    .find(|failure| failure.code() == class)
                    .map(ProtocolFailure::group)
            })
            .unwrap_or(ErrorGroup::Internal);
        Self(Box::new(Inner {
            code: ErrorCode::new(&code).unwrap_or(ErrorCode::INTERNAL),
            group,
            retryable,
            message,
            data: details,
            answered: true,
            source: None,
        }))
    }

    /// Whether the engine answered the call with it, rather than the
    /// Connection failing it, as on a timeout.
    pub(crate) fn is_answer(&self) -> bool {
        self.0.answered
    }

    /// The JSON-RPC error object it answers an engine's request with.
    pub(crate) fn to_object(&self) -> ErrorObject {
        ErrorObject {
            code: self.0.group.json_rpc_class(),
            message: self.0.message.clone(),
            data: ErrorData {
                code: self.0.code.as_str().to_owned(),
                retryable: self.0.retryable,
                details: self.0.data.clone(),
            },
        }
    }

    /// The Connection, or the value the call was made on, is closed, so
    /// the call was never answered: `closed`. A Product crate's linked
    /// Connection fails with it too.
    #[must_use]
    pub fn closed() -> Self {
        Self::of_runtime(
            &CLOSED,
            "the Connection, or the value the call was made on, is closed",
            "Connect again, or open the value again: a closed one never reopens.",
        )
    }

    /// The engine ended the Connection, as a Daemon that dropped it, before
    /// it answered the call: `closed`, since the Connection is.
    pub(crate) fn dropped() -> Self {
        Self::of_runtime(
            &CLOSED,
            "the engine ended the Connection before it answered",
            "Connect again: a closed Connection never reopens.",
        )
    }

    /// The error, which the runtime raised, as the call `operation`, a
    /// catalog name, fails with it (ADRs 0079 and 0099): its message
    /// headed by the operation in Rust's spelling, and `operation` set to
    /// the catalog's name. An error the engine answered with, or one that
    /// already names its operation, stays as it is.
    #[must_use]
    pub(crate) fn in_call(mut self, operation: &str) -> Self {
        if self.0.answered || self.0.data.contains_key("operation") {
            return self;
        }
        self.0.message = format!(
            "{}: {}",
            naming::operation(Form::Rust, operation),
            self.0.message
        );
        self.0
            .data
            .insert("operation".to_owned(), operation.to_owned().into());
        self
    }

    /// The call's timeout ran out before its reply came:
    /// `deadline_exceeded`. A Product crate's linked Connection fails a
    /// close that runs out with it too.
    #[must_use]
    pub fn timed_out() -> Self {
        Self::of(&DEADLINE_EXCEEDED, "the call timed out".to_owned())
    }

    /// The transport could not reach the engine: `connection_lost`, with
    /// the OS's error as its cause.
    pub(crate) fn unreachable(source: io::Error) -> Self {
        Self::of(
            &CONNECTION_LOST,
            format!("could not reach the engine: {source}"),
        )
        .caused_by(source)
    }

    /// Another stream already serves the engine's requests of `method`, a
    /// bug in the program the catalog has no code of its own for:
    /// `internal`.
    pub(crate) fn already_served(method: &str) -> Self {
        Self::of(
            &INTERNAL,
            format!("the engine's requests of `{method}` are already served"),
        )
    }

    /// A Session's stream of `event` held as many unread events as its
    /// queue takes, then lost `lost` more of its own (ADR 0081):
    /// `listener_events_lost`, whose details name both.
    pub(crate) fn listener_events_lost(event: &str, lost: u64) -> Self {
        let mut error = Self::of_runtime(
            &goodfetch_protocol::events::LISTENER_EVENTS_LOST,
            &format!(
                "the stream of `{event}` held as many unread events as its queue takes, so it lost {lost} more and ended"
            ),
            "Read the stream sooner, or raise `max_listener_queue` where the connection opens, then ask for the events again.",
        );
        let details = goodfetch_protocol::events::ListenerEventsLostDetails {
            event: event.to_owned(),
            lost,
        };
        if let Ok(Value::Object(details)) = serde_json::to_value(details) {
            error.0.data.extend(details);
        }
        error
    }

    /// A call's params or result did not fit its type: `internal`, with
    /// serde's error as its cause.
    pub(crate) fn shape(source: serde_json::Error) -> Self {
        Self::of(
            &INTERNAL,
            format!("a message did not fit its type: {source}"),
        )
        .caused_by(source)
    }

    /// The file the engine keeps a response's body in could not be read
    /// whole, or no longer holds the body's bytes: `body_unavailable`, with
    /// the OS's error as its cause.
    pub(crate) fn body_unreadable(path: &Path, source: io::Error) -> Self {
        Self::of(
            &BODY_UNAVAILABLE,
            format!(
                "could not read the response's body from `{}`: {source}",
                path.display()
            ),
        )
        .caused_by(source)
    }

    /// A response's body is not JSON, or not the JSON the program reads it
    /// as: `response_not_json` (ADR 0085), with serde's error as its cause.
    pub(crate) fn response_not_json(url: String, source: serde_json::Error) -> Self {
        let message = format!("the body of {url} is not the JSON asked for: {source}");
        let mut error = Self::of(&RESPONSE_NOT_JSON, message).caused_by(source);
        error.0.data.insert("url".to_owned(), url.into());
        error
    }

    /// The program's path cannot be written for a reason the path causes:
    /// `path_not_writable` (ADR 0116), with the OS's error as its cause.
    pub(crate) fn path_not_writable(
        details: &PathNotWritableDetails,
        message: String,
        cause: Option<io::Error>,
    ) -> Self {
        Self::of(&PATH_NOT_WRITABLE, message).with_file_failure(details, None, cause)
    }

    /// The program's storage failed while a file was written to its path:
    /// `file_write_failed`, side `program` (ADR 0116), with the OS's error
    /// as its cause.
    pub(crate) fn file_write_failed(
        details: &FileWriteFailedDetails,
        message: String,
        fix: String,
        cause: Option<io::Error>,
    ) -> Self {
        Self::of(&FILE_WRITE_FAILED, message).with_file_failure(details, Some(fix), cause)
    }

    fn with_file_failure(
        mut self,
        details: &impl Serialize,
        fix: Option<String>,
        cause: Option<io::Error>,
    ) -> Self {
        if let Ok(Value::Object(details)) = serde_json::to_value(details) {
            self.0.data.extend(details);
        }
        if let Some(fix) = fix {
            self.0.data.insert("fix".to_owned(), fix.into());
        }
        if let Some(cause) = cause {
            self = self.caused_by(cause);
        }
        self
    }

    /// A failure the runtime raises itself, of the code `defined`, in ADR
    /// 0079's layout: what failed, then `fix`, which its field holds too.
    /// [`Error::in_call`] heads it with the call's operation.
    fn of_runtime(defined: &ErrorCodeDefinition, what_failed: &str, fix: &str) -> Self {
        let mut error = Self::of(defined, format!("{what_failed}. {fix}"));
        error.0.data.insert("fix".to_owned(), fix.into());
        error
    }

    /// A failure of the SDK's own of the code `defined`.
    fn of(defined: &ErrorCodeDefinition, message: String) -> Self {
        Self(Box::new(Inner {
            code: ErrorCode::new(defined.name)
                .expect("invariant: codegen checks every catalog code fits an ErrorCode"),
            group: defined.group,
            retryable: defined.retryable,
            message,
            data: Map::new(),
            answered: false,
            source: None,
        }))
    }

    fn caused_by(mut self, source: impl Into<BoxError>) -> Self {
        self.0.source = Some(source.into());
        self
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.0.data.get(key).and_then(Value::as_str)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.message)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("code", &self.0.code)
            .field("group", &self.0.group)
            .field("retryable", &self.0.retryable)
            .field("message", &self.0.message)
            .field("data", &self.0.data)
            .field("source", &self.0.source)
            .finish()
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0
            .source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

/// C-SEND-SYNC and C-GOOD-ERR: an error crosses tasks and threads, and
/// converts into a [`BoxError`] with `?` (ADR 0109).
const _: () = {
    const fn send_sync_static<T: std::error::Error + Send + Sync + 'static>() {}
    send_sync_static::<Error>();
};

#[cfg(test)]
mod tests;
