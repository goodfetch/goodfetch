//! A failure a call answers with (ADR 0038): an Error code of the catalog
//! with its message and details, or a failure of the protocol itself.
//! Handlers and the modules beneath them build one; only the RPC layer
//! turns it into the wire's error object, in the form of the Connection it
//! answers (ADR 0099), the names [`named`] marks in its text included.

use std::fmt;

use goodfetch_protocol::catalog::ErrorCodeDefinition;
use goodfetch_protocol::connection::Form;
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject, ProtocolFailure};
use goodfetch_protocol::naming;
use serde::Serialize;
use serde_json::{Map, Value};

pub mod named;

/// A failure a call answers with: an Error code of the catalog, with its
/// message and details, or a failure of the protocol itself.
#[derive(Clone)]
pub struct CallError(Box<Failure>);

#[derive(Clone)]
enum Failure {
    Code {
        definition: &'static ErrorCodeDefinition,
        /// Its message, each name [`named`] marked in it canonical.
        message: String,
        /// Its details, each name marked in them canonical.
        details: Map<String, Value>,
        /// Its message and details as written, when they mark a name,
        /// which the RPC layer renders in its Connection's form.
        marked: Option<Box<Marked>>,
    },
    Protocol {
        failure: ProtocolFailure,
        message: String,
    },
}

/// A failure's message and details as written, marks and all.
#[derive(Clone)]
struct Marked {
    message: String,
    details: Map<String, Value>,
}

impl Marked {
    /// Its message and details with each mark rendered in `form`.
    fn rendered(&self, form: Form) -> (String, Map<String, Value>) {
        let details = self
            .details
            .iter()
            .map(|(key, value)| (key.clone(), named::render_value(value, form)))
            .collect();
        (named::render(&self.message, form).into_owned(), details)
    }
}

impl CallError {
    /// A failure with an Error code of the catalog and the code's details,
    /// which serialize to a JSON object. Its message, and a text of its
    /// details such as a `fix`, may mark the names its Client spells in
    /// its own form (ADR 0099) with [`named`].
    pub fn new(
        definition: &'static ErrorCodeDefinition,
        message: impl Into<String>,
        details: &impl Serialize,
    ) -> Self {
        let message = message.into();
        let details = match serde_json::to_value(details) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        };
        let (message, details, marked) =
            if named::marked(&message) || details.values().any(named::marked_value) {
                let marked = Marked { message, details };
                let (message, details) = marked.rendered(Form::Catalog);
                (message, details, Some(Box::new(marked)))
            } else {
                (message, details, None)
            };
        Self(Box::new(Failure::Code {
            definition,
            message,
            details,
            marked,
        }))
    }

    /// A failure of the protocol itself, such as params that are not the
    /// operation's: a name marked in its message reads canonically.
    pub fn protocol(failure: ProtocolFailure, message: impl Into<String>) -> Self {
        let message = message.into();
        Self(Box::new(Failure::Protocol {
            failure,
            message: named::render(&message, Form::Catalog).into_owned(),
        }))
    }

    /// The failure as the call `operation`, a catalog name, answers it
    /// (ADRs 0079 and 0099): its message headed by the operation in the
    /// Client's `form`, and `operation` in its details by its canonical
    /// name. A failure that already names its operation, and a protocol
    /// failure, stay as they are.
    #[must_use]
    pub fn in_call(mut self, operation: &str, form: Form) -> Self {
        if let Failure::Code {
            message,
            details,
            marked,
            ..
        } = &mut *self.0
            && !details.contains_key("operation")
        {
            let spelled = naming::operation(form, operation);
            let head = |message: &mut String, details: &mut Map<String, Value>| {
                *message = format!("{spelled}: {message}");
                details.insert("operation".to_owned(), Value::from(operation));
            };
            head(message, details);
            if let Some(marked) = marked {
                head(&mut marked.message, &mut marked.details);
            }
        }
        self
    }

    /// Its Error code, or the protocol failure's name.
    pub fn code(&self) -> &'static str {
        match &*self.0 {
            Failure::Code { definition, .. } => definition.name,
            Failure::Protocol { failure, .. } => failure.name(),
        }
    }

    /// Its Error code's details; none for a protocol failure.
    pub fn details(&self) -> Option<&Map<String, Value>> {
        match &*self.0 {
            Failure::Code { details, .. } => Some(details),
            Failure::Protocol { .. } => None,
        }
    }

    /// What it says, for people.
    pub fn message(&self) -> &str {
        match &*self.0 {
            Failure::Code { message, .. } | Failure::Protocol { message, .. } => message,
        }
    }

    /// Whether a retry may succeed, as its Error code says; never for a
    /// protocol failure.
    pub fn retryable(&self) -> bool {
        match &*self.0 {
            Failure::Code { definition, .. } => definition.retryable,
            Failure::Protocol { .. } => false,
        }
    }

    /// Its numeric code on the wire: a predefined JSON-RPC code or its
    /// group's class.
    pub(crate) fn wire_code(&self) -> i32 {
        match &*self.0 {
            Failure::Code { definition, .. } => definition.group.json_rpc_class(),
            Failure::Protocol { failure, .. } => failure.code(),
        }
    }

    /// Its numeric class on the wire, whether a retry may succeed and its
    /// details, for the RPC layer.
    pub(crate) fn wire_parts(&self) -> (i32, bool, Map<String, Value>) {
        match &*self.0 {
            Failure::Code {
                definition,
                details,
                ..
            } => (
                definition.group.json_rpc_class(),
                definition.retryable,
                details.clone(),
            ),
            Failure::Protocol { failure, .. } => (failure.code(), false, Map::new()),
        }
    }

    /// Its error object, as the wire carries it and as a CLI command that
    /// fails before any call prints it: each name it marks canonical.
    pub fn to_error_object(&self) -> ErrorObject {
        let (code, retryable, details) = self.wire_parts();
        ErrorObject {
            code,
            message: self.message().to_owned(),
            data: ErrorData {
                code: self.code().to_owned(),
                retryable,
                details,
            },
        }
    }

    /// Its error object as a Connection whose Client writes `form` receives
    /// it (ADR 0099): each name its message and its details' texts mark
    /// spelled in that form, its structured fields, such as `limit`'s
    /// `key`, canonical.
    pub fn to_error_object_in(&self, form: Form) -> ErrorObject {
        let mut object = self.to_error_object();
        if let Failure::Code {
            marked: Some(marked),
            ..
        } = &*self.0
        {
            (object.message, object.data.details) = marked.rendered(form);
        }
        object
    }
}

impl fmt::Debug for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (_, retryable, details) = self.wire_parts();
        formatter
            .debug_struct("CallError")
            .field("code", &self.code())
            .field("message", &self.message())
            .field("retryable", &retryable)
            .field("details", &details)
            .finish()
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for CallError {}

#[cfg(test)]
mod tests;
