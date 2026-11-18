//! Why a Connection or one of its calls failed. The error group classes and
//! the classes of Collie's Error codes replace [`Error::Failed`]'s one shape
//! with the ticket that adds them (spec #606).

use goodfetch_protocol::jsonrpc::ErrorObject;

/// Why a Connection or one of its calls failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The transport could not reach the engine.
    #[error("could not reach the engine: {0}")]
    Unreachable(#[source] std::io::Error),
    /// The Connection is closed, by `close()` or by its engine, so the call
    /// was never answered.
    #[error("the Connection is closed")]
    Closed,
    /// The call's timeout ran out before its reply came.
    #[error("the call timed out")]
    TimedOut,
    /// The engine answered the call with a failure.
    #[error("{}: {}", .0.data.code, .0.message)]
    Failed(Box<ErrorObject>),
    /// A call's params or result did not fit its type.
    #[error("a message did not fit its type: {0}")]
    Shape(#[source] serde_json::Error),
    /// The engine the SDK runs in the program's own process, its linked
    /// engine, could not start or failed (ADR 0087).
    #[error("the linked engine failed: {0}")]
    Engine(#[source] Box<dyn std::error::Error + Send + Sync>),
}
