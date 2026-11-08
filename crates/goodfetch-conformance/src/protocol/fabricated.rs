//! A generated refusal case's fabricated `hello` sent to a running engine
//! (ADR 0066): the harness announces the Product the case needs (ADR 0095),
//! sends a call right behind the `hello`, and reports the first message the
//! engine writes. A refusal must be that first message, and the Connection
//! must then close before any other: the harness never judges a refusal by
//! a time budget, only by what comes first.

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::{HELLO, HelloParams, HelloResult, Surface};
use goodfetch_protocol::engine::ENGINE_INFO;
use goodfetch_protocol::formats::HelloFrom;
use goodfetch_protocol::refusals::RefusalCase;
use serde_json::{Value, json};

use super::connection::{Connection, Gone};
use super::stop;
use super::target::{self, EngineTarget, START_DEADLINE, StartError};

/// What a running engine answered a fabricated `hello`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FabricatedHelloOutcome {
    /// Its first message accepted it.
    Accepted {
        /// The protocol version the reply names.
        protocol_version: ProtocolVersion,
        /// The engine's Product.
        product: Product,
    },
    /// Its first message refused it.
    Refused {
        /// The refusal's Error code.
        error_code: String,
        /// Whether the engine then closed the Connection before any other
        /// message.
        closed: bool,
    },
}

/// Why a fabricated `hello` got no outcome.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FabricatedHelloError {
    /// The target could not be reached.
    #[error(transparent)]
    Start(#[from] StartError),
    /// The case fabricates no `hello` an engine answers.
    #[error("the case fabricates no `hello` an engine answers: {reason}")]
    NotForAnEngine {
        /// Why.
        reason: String,
    },
    /// The engine wrote nothing.
    #[error("the engine wrote nothing after the `hello`: {reason}")]
    Unanswered {
        /// Why.
        reason: String,
    },
    /// The Daemon accepted the `hello` but did not take the harness's
    /// `daemon.stop`.
    #[error("the Daemon accepted the `hello` but did not take the harness's stop: {reason}")]
    NotStopped {
        /// Why.
        reason: String,
    },
    /// The engine's first message is no reply to the `hello`.
    #[error("the engine's first message is no reply to the `hello`: {message}")]
    NotTheReply {
        /// The message.
        message: Value,
    },
}

/// Sends `case`'s fabricated `hello` to `target`, with a call behind it, and
/// reports the first message the engine writes.
///
/// # Errors
///
/// When the target cannot be reached, the case fabricates no `hello` an
/// engine answers, or the engine's first message is no reply to it.
pub fn send_fabricated_hello(
    target: &EngineTarget,
    case: &RefusalCase,
) -> Result<FabricatedHelloOutcome, FabricatedHelloError> {
    let Some(hello) = &case.hello else {
        return Err(FabricatedHelloError::NotForAnEngine {
            reason: "it is a file's".to_owned(),
        });
    };
    let surface = match hello.from {
        HelloFrom::Client => Surface::Rust,
        HelloFrom::BrowserHost => Surface::BrowserHost,
        HelloFrom::GroupMember => {
            return Err(FabricatedHelloError::NotForAnEngine {
                reason: "a Group member opens the peer protocol".to_owned(),
            });
        }
    };
    let mut connection = Connection::new(target::Started::start(target)?);
    let params = HelloParams {
        protocol_version: case.found,
        needs: hello.needs,
        ..connection.hello_params(surface)
    };
    let (id, first) = with_a_call_behind(&mut connection, &params).map_err(|why| {
        FabricatedHelloError::Unanswered {
            reason: why.to_string(),
        }
    })?;
    let outcome = match reading(&first, id) {
        Some(FabricatedHelloOutcome::Refused { error_code, .. }) => {
            FabricatedHelloOutcome::Refused {
                error_code,
                closed: connection.closes(),
            }
        }
        Some(accepted) => {
            let stopped = stop(&mut connection, target.mode);
            connection.close();
            stopped.map_err(|reason| FabricatedHelloError::NotStopped { reason })?;
            return Ok(accepted);
        }
        None => {
            connection.close();
            return Err(FabricatedHelloError::NotTheReply { message: first });
        }
    };
    connection.close();
    Ok(outcome)
}

/// Sends `params` as the Connection's `hello` with a call right behind it,
/// and reads the first message the engine writes: the `hello`'s ID and that
/// message.
pub(super) fn with_a_call_behind(
    connection: &mut Connection,
    params: &HelloParams,
) -> Result<(i64, Value), Gone> {
    let params = serde_json::to_value(params).expect("invariant: `hello` serializes");
    let id = connection.request(HELLO, &params)?;
    connection.request(ENGINE_INFO.name, &json!({}))?;
    Ok((id, connection.next_within(START_DEADLINE)?))
}

/// What `message` says of the `hello` sent as `id`, if it replies to it.
fn reading(message: &Value, id: i64) -> Option<FabricatedHelloOutcome> {
    if message.get("method").is_some() || message.get("id") != Some(&json!(id)) {
        return None;
    }
    if let Some(error) = message.get("error") {
        return Some(FabricatedHelloOutcome::Refused {
            error_code: error["data"]["code"].as_str()?.to_owned(),
            closed: false,
        });
    }
    let result: HelloResult = serde_json::from_value(message.get("result")?.clone()).ok()?;
    Some(FabricatedHelloOutcome::Accepted {
        protocol_version: result.protocol_version,
        product: result.product,
    })
}
