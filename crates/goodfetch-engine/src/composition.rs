//! The composition (ADR 0087): each domain crate exports one
//! [`Contribution`] listing what it adds to the engine, and each Product's
//! executable crate passes the ones it links to the engine at start, a list
//! in its `main`. Nothing registers itself by being linked. Today a
//! contribution adds operation handlers and the tables it keeps in every
//! run file; settings keys, Plugin kinds and Hook stages, Usage kinds and
//! Limit scopes, Notifications and gates in admission join it with the
//! tickets that build them.
//!
//! A handler answers a value or a [`CallError`], which only the RPC layer
//! turns into the wire's error object.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::catalog::{ErrorCodeDefinition, Operation};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::client::Client;
use crate::product::Identity;
use crate::run_state::RunStateTables;
use crate::runs::Runs;

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

    /// What it says, for people.
    pub fn message(&self) -> &str {
        match &*self.0 {
            Failure::Code { message, .. } | Failure::Protocol { message, .. } => message,
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

/// What a handler is given beside its params.
#[derive(Clone)]
pub struct CallContext {
    identity: Arc<Identity>,
    token: CancellationToken,
    client: Client,
    runs: Runs,
}

impl CallContext {
    pub(crate) fn new(
        identity: Arc<Identity>,
        token: CancellationToken,
        client: Client,
        runs: Runs,
    ) -> Self {
        Self {
            identity,
            token,
            client,
            runs,
        }
    }

    /// The Client of the call's Connection, to which a Run started by the
    /// call sends its handler requests.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The engine's Runs.
    pub fn runs(&self) -> &Runs {
        &self.runs
    }

    /// The running engine's Product and version.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Cancelled when the call is: a handler that waits watches it at each
    /// cancellation point.
    pub fn cancellation(&self) -> &CancellationToken {
        &self.token
    }
}

type Answer = Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>>;

/// The handler of one operation of the catalog.
#[derive(Clone)]
pub struct Handler {
    operation: &'static Operation,
    call: Arc<dyn Fn(Value, CallContext) -> Answer + Send + Sync>,
}

impl Handler {
    /// Handles `operation` with `handle`, which takes the operation's params
    /// type and answers its result type; params that do not read as `P`
    /// answer `invalid_params`.
    pub fn new<P, R, F, Fut>(operation: &'static Operation, handle: F) -> Self
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + 'static,
        F: Fn(P, CallContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R, CallError>> + Send + 'static,
    {
        let handle = Arc::new(handle);
        let call = move |params: Value, context: CallContext| -> Answer {
            let read = serde_json::from_value::<P>(params);
            let handle = Arc::clone(&handle);
            Box::pin(async move {
                let params = read.map_err(|error| {
                    CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
                })?;
                let result = handle(params, context).await?;
                serde_json::to_value(result).map_err(|error| {
                    CallError::protocol(ProtocolFailure::InternalError, error.to_string())
                })
            })
        };
        Self {
            operation,
            call: Arc::new(call),
        }
    }

    /// The operation it handles.
    pub fn operation(&self) -> &'static Operation {
        self.operation
    }

    pub(crate) fn call(&self, params: Value, context: CallContext) -> Answer {
        (self.call)(params, context)
    }
}

/// What one domain crate adds to the engine.
#[derive(Clone)]
pub struct Contribution {
    name: &'static str,
    handlers: Vec<Handler>,
    run_state_tables: Vec<RunStateTables>,
}

impl Contribution {
    /// An empty contribution of the crate or concern `name`.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            handlers: Vec::new(),
            run_state_tables: Vec::new(),
        }
    }

    /// The same, adding `handler`.
    #[must_use]
    pub fn with_handler(mut self, handler: Handler) -> Self {
        self.handlers.push(handler);
        self
    }

    /// The same, adding `tables` to every run file (ADR 0087).
    #[must_use]
    pub fn with_run_state_tables(mut self, tables: RunStateTables) -> Self {
        self.run_state_tables.push(tables);
        self
    }

    /// Who contributes it.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// The contributions an engine starts with.
#[derive(Clone, Default)]
pub struct Composition {
    contributions: Vec<Contribution>,
}

/// A composition no engine can start from.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CompositionError {
    /// Two handlers for one operation.
    #[error("`{operation}` has a handler from both `{first}` and `{second}`")]
    DuplicateHandler {
        /// The operation.
        operation: &'static str,
        /// The first contribution that handles it.
        first: &'static str,
        /// The second.
        second: &'static str,
    },
}

impl Composition {
    /// The same, adding `contribution`.
    #[must_use]
    pub fn with(mut self, contribution: Contribution) -> Self {
        self.contributions.push(contribution);
        self
    }

    /// The name of each contribution, in order.
    pub fn contributions(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.contributions.iter().map(Contribution::name)
    }

    /// The operation of each handler, once per handler, so a test sees an
    /// operation handled twice.
    pub fn operations(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.contributions
            .iter()
            .flat_map(|contribution| &contribution.handlers)
            .map(|handler| handler.operation.name)
    }

    /// The tables every contribution adds to every run file, in order.
    pub(crate) fn run_state_tables(&self) -> Vec<RunStateTables> {
        self.contributions
            .iter()
            .flat_map(|contribution| contribution.run_state_tables.iter().copied())
            .collect()
    }

    /// Each operation's one handler.
    pub(crate) fn handlers(&self) -> Result<BTreeMap<&'static str, Handler>, CompositionError> {
        let mut handlers: BTreeMap<&'static str, (&'static str, Handler)> = BTreeMap::new();
        for contribution in &self.contributions {
            for handler in &contribution.handlers {
                let operation = handler.operation.name;
                if let Some((first, _)) = handlers.get(operation) {
                    return Err(CompositionError::DuplicateHandler {
                        operation,
                        first,
                        second: contribution.name,
                    });
                }
                handlers.insert(operation, (contribution.name, handler.clone()));
            }
        }
        Ok(handlers
            .into_iter()
            .map(|(operation, (_, handler))| (operation, handler))
            .collect())
    }
}
