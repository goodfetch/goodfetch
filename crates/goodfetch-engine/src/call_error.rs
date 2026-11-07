//! A failure a call answers with (ADR 0038): an Error code of the catalog
//! with its message and details, or a failure of the protocol itself.
//! Handlers and the modules beneath them build one; only the RPC layer
//! turns it into the wire's error object.

use std::fmt;

use goodfetch_protocol::catalog::ErrorCodeDefinition;
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject, ProtocolFailure};
use serde::Serialize;
use serde_json::{Map, Value};

/// A failure a call answers with: an Error code of the catalog, with its
/// message and details, or a failure of the protocol itself.
#[derive(Clone)]
pub struct CallError(Box<Failure>);

#[derive(Clone)]
enum Failure {
    Code {
        definition: &'static ErrorCodeDefinition,
        message: String,
        details: Map<String, Value>,
    },
    Protocol {
        failure: ProtocolFailure,
        message: String,
    },
}

impl CallError {
    /// A failure with an Error code of the catalog and the code's details,
    /// which serialize to a JSON object.
    pub fn new(
        definition: &'static ErrorCodeDefinition,
        message: impl Into<String>,
        details: &impl Serialize,
    ) -> Self {
        let details = match serde_json::to_value(details) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        };
        Self(Box::new(Failure::Code {
            definition,
            message: message.into(),
            details,
        }))
    }

    /// A failure of the protocol itself, such as params that are not the
    /// operation's.
    pub fn protocol(failure: ProtocolFailure, message: impl Into<String>) -> Self {
        Self(Box::new(Failure::Protocol {
            failure,
            message: message.into(),
        }))
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
    /// fails before any call prints it.
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
