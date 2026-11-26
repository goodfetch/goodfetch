//! Calls an Attached engine forwards to its major's Daemon, which holds
//! what they act on (spec 589): the Schedule store is the Daemon's, so a
//! `schedule.*` call made on an Attached engine goes to the Daemon, which
//! the engine starts when none runs, and the Daemon's answer, a result or
//! a failure, is the call's. A Daemon, and an engine that cannot start
//! one, answers the call itself.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::catalog::{Operation, Product};
use goodfetch_protocol::connection::{CONNECTION_LOST, Form, HelloParams, Surface};
use goodfetch_protocol::daemon::{DAEMON_UNREACHABLE, DaemonEndpoint, DaemonUnreachableDetails};
use goodfetch_protocol::entries;
use goodfetch_protocol::jsonrpc::{ErrorObject, META, ProtocolFailure};
use serde_json::{Value, json};

use super::{NoAnswer, ToOwner};
use crate::call_error::CallError;
use crate::engine_context::ToTheDaemon;
use crate::tasks::TaskOwner;

/// What reaches the Daemon of the engine's major, starting it when none
/// runs: the executable's own way of starting its Daemon, which an
/// Attached engine is given (ADR 0002).
pub trait DaemonReach: Send + Sync {
    /// Where the running Daemon listens and the token its `hello` takes,
    /// once it is started if it was not; or why it cannot be reached,
    /// such as `daemon_unreachable`. It blocks while the Daemon starts.
    ///
    /// # Errors
    ///
    /// The failure the call answers: no Daemon could be started or reached.
    fn running_or_started(&self) -> Result<(DaemonEndpoint, String), ErrorObject>;
}

/// What forwards an Attached engine's calls to its major's Daemon: the
/// Daemon `reach` finds or starts, greeted and called as `owner` greets
/// and calls a Run's owner.
pub(crate) struct ThroughTheReach {
    reach: Arc<dyn DaemonReach>,
    owner: Arc<ToOwner>,
}

impl ThroughTheReach {
    /// Forwards through `reach`, saying `hello` and calling as `owner` does.
    pub(crate) fn new(reach: Arc<dyn DaemonReach>, owner: Arc<ToOwner>) -> Self {
        Self { reach, owner }
    }
}

impl ToTheDaemon for ThroughTheReach {
    /// The call waits as long as the Daemon does: the Daemon bounds it by
    /// the call's own `timeout`, which `params` carries.
    fn forward(
        &self,
        tasks: Arc<TaskOwner>,
        operation: &'static Operation,
        mut params: Value,
        (surface, form): (Surface, Form),
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError>> + Send>> {
        let reach = Arc::clone(&self.reach);
        let owner = Arc::clone(&self.owner);
        Box::pin(async move {
            let reached = tasks.blocking(move || reach.running_or_started()).await;
            let (endpoint, token) = reached.map_err(|failure| answered_failure(&failure))?;
            // The forwarded call carries its own trace context.
            if let Value::Object(members) = &mut params {
                members.remove(META);
            }
            let hello = HelloParams {
                form: Some(form),
                ..owner.hello_from(surface, token, Product::Collie)
            };
            match owner
                .call(&endpoint, &hello, operation.name, &params, None)
                .await
            {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(failure)) => Err(answered_failure(&failure)),
                Err(reason) => Err(unanswered(operation, &reason)),
            }
        })
    }
}

/// The failure the Daemon answered, or answered before any call: its
/// Error code with its details, or the protocol failure it names.
pub(crate) fn answered_failure(failure: &ErrorObject) -> CallError {
    let code = failure.data.code.as_str();
    if let Some(definition) = entries::error_code(code) {
        return CallError::new(definition, failure.message.clone(), &failure.data.details);
    }
    let protocol = ProtocolFailure::ALL
        .into_iter()
        .find(|protocol| protocol.name() == code)
        .unwrap_or(ProtocolFailure::InternalError);
    CallError::protocol(protocol, failure.message.clone())
}

/// The failure of a call the Daemon gave no answer to: `daemon_unreachable`
/// when it never took the call, `connection_lost` when it went away after
/// the call was sent, so whether it took effect is unknown.
fn unanswered(operation: &'static Operation, reason: &NoAnswer) -> CallError {
    match reason {
        NoAnswer::Unconnected(_)
        | NoAnswer::HelloRefused(_)
        | NoAnswer::HelloUnanswered
        | NoAnswer::NotSent { .. } => CallError::new(
            &DAEMON_UNREACHABLE,
            format!(
                "The Daemon that keeps `{}` could not be reached: {reason}.",
                operation.name
            ),
            &DaemonUnreachableDetails {
                fix: "Check `daemon status`, then make the call again.".to_owned(),
            },
        ),
        NoAnswer::CallUnanswered(_)
        | NoAnswer::Unread(_)
        | NoAnswer::Closed
        | NoAnswer::Failed(_)
        | NoAnswer::NoReport(_) => CallError::new(
            &CONNECTION_LOST,
            format!(
                "The Connection to the Daemon dropped before it answered `{}`, so whether it took effect is unknown: {reason}.",
                operation.name
            ),
            &json!({
                "fix": "Check `daemon status` and whether the call took effect before making it again."
            }),
        ),
    }
}
