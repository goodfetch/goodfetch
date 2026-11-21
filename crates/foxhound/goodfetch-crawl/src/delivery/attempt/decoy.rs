//! The ending of a crawl request whose page is a decoy (ADR 0035):
//! `blocked` with `decoy`, never retried, escalated or followed, its
//! details the protocol's `blocked` details. A page marked `noindex` that
//! failed its Page check was fetched, so its details name its status; a
//! page reached through a suspected trap link was never fetched, so its
//! status is `null`.

use goodfetch_protocol::foxhound::block::{BlockCause, BlockedDetails};
use goodfetch_protocol::page::BLOCKED;
use serde_json::Value;

use super::{Answer, AttemptFailure};

/// The ending of the request whose page at `url`, answered with `status`,
/// is marked `noindex` and failed its Page check.
pub(super) fn noindexed(url: &str, status: u16) -> Answer {
    decoy(
        format!(
            "The page `{url}`, answered with {status}, is marked `noindex` and failed its Page check: a decoy, which is never retried, escalated or followed."
        ),
        url,
        Some(status),
    )
}

/// The ending of the request at `url` on a browser tier when it was added
/// through a link its served document's markup hides: a browser tier never
/// follows a link a person cannot see, so its page, never fetched, is a
/// decoy.
pub(super) fn reached_through_trap(url: &str) -> Answer {
    decoy(
        format!(
            "The page `{url}` was reached through a link its page's markup hides from people, a trap: a browser never follows one, so it is a decoy, never fetched."
        ),
        url,
        None,
    )
}

fn decoy(message: String, url: &str, status: Option<u16>) -> Answer {
    let details = BlockedDetails {
        cause: BlockCause::Decoy,
        url: url.to_owned(),
        status,
        retry_after: None,
    };
    let Ok(Value::Object(details)) = serde_json::to_value(&details) else {
        unreachable!("invariant: `blocked` details serialize to an object")
    };
    Answer::failed(AttemptFailure::entered(
        BLOCKED.name.to_owned(),
        false,
        message,
        details,
    ))
}

#[cfg(test)]
mod tests;
