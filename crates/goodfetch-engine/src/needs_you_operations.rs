//! The `needs_you.*` operations every Product carries (spec #592):
//! `needs_you.list` reads a page of "Needs you", and `needs_you.dismiss`
//! hides an item whose source allows it, both through the engine's
//! "Needs you" driver.

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::needs_you::{
    NEEDS_YOU_DISMISS, NEEDS_YOU_LIST, NeedsYouDismissParams, NeedsYouDismissed, NeedsYouList,
    NeedsYouListParams,
};

use crate::composition::{CallContext, CallError, Handler};
use crate::needs_you::NeedsYouError;

/// The `needs_you.*` handlers.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&NEEDS_YOU_LIST, list),
        Handler::new(&NEEDS_YOU_DISMISS, dismiss),
    ]
}

fn failed(error: &NeedsYouError) -> CallError {
    let failure = match error {
        NeedsYouError::Cursor(_) => ProtocolFailure::InvalidParams,
        NeedsYouError::Stopped | NeedsYouError::Index(_) => ProtocolFailure::InternalError,
    };
    CallError::protocol(failure, error.to_string())
}

async fn list(params: NeedsYouListParams, context: CallContext) -> Result<NeedsYouList, CallError> {
    context
        .runs()
        .needs_you()
        .list(params)
        .await
        .map_err(|error| failed(&error))
}

async fn dismiss(
    params: NeedsYouDismissParams,
    context: CallContext,
) -> Result<NeedsYouDismissed, CallError> {
    let dismissed = context
        .runs()
        .needs_you()
        .dismiss(params.item)
        .await
        .map_err(|error| failed(&error))?;
    Ok(NeedsYouDismissed { dismissed })
}
