//! The call and its context.

use std::error::Error;

use futures_util::stream::BoxStream;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::ledger::Reservation;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::model::{ModelRequest, ModelResponsePart};
use tokio_util::sync::CancellationToken;

/// A Model provider: what answers the engine's model calls.
pub trait ModelProvider: Send + Sync {
    /// Asks the model `request` names, within `context`. The stream yields
    /// the answer's text in pieces as they arrive, then the whole
    /// [`ModelResponse`](goodfetch_protocol::model::ModelResponse) with
    /// its token counts, last; an `Err` ends it with the provider's
    /// failure, which the driver classifies (ADR 0129).
    fn call<'a>(
        &'a self,
        request: &'a ModelRequest,
        context: CallContext<'a>,
    ) -> BoxStream<'a, Result<ModelResponsePart, Box<dyn Error + Send + Sync>>>;
}

/// What one model call carries besides its request: the engine's own
/// values for it.
#[derive(Clone, Copy, Debug)]
pub struct CallContext<'a> {
    /// The Run the call is for.
    pub run: &'a Run,
    /// The worst case the driver reserved for the call, which it settles
    /// once the provider has said what the call used.
    pub reservation: &'a Reservation,
    /// When the call must have answered; none when its timeout is unset.
    pub deadline: Option<&'a Deadline>,
    /// Cancelled when the call is no longer wanted: the provider stops and
    /// ends its stream.
    pub cancellation: &'a CancellationToken,
}
