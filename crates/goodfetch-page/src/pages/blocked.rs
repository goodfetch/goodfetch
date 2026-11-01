//! A Block cause on a Session a program or an Agent opened (spec #577's
//! `sessions`, ticket #711; ADRs 0088, 0092, 0098 and 0118): the document a
//! page's navigation reached is classified by its Product's Block
//! classification, and a Block cause raises `blocked` with the cause, the
//! URL, the status and the time the site's `Retry-After` asks for. The
//! Session stays open with its cookies: the engine never replaces it, never
//! waits on the `Retry-After` and writes nothing to Politeness's domain
//! limits, so the program decides whether to wait, open another Session or
//! pause for a person. A Challenge raises `blocked` too, until area 8's
//! Challenge ladder runs first.
//!
//! A document is read only when the classifier looks into documents of its
//! response's status, and then only for the markers it names, found in the
//! page's own serialization, so a page's markup never crosses the debugging
//! pipe whole.

use std::time::SystemTime;

use goodfetch_engine::CallError;
use goodfetch_engine::retry_after::{self, RetryAfter};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::page::{BLOCKED, BlockedDetails, PageId, PageResponse};
use serde_json::json;

use crate::blocks::{Block, BlockClassifier};
use crate::consent::Walls;
use crate::driver::PageDriver;
use crate::page_events::PageEvents;
use crate::proxy;

/// What the page's document is read for: which of the markers asked for
/// its serialization holds, lowercase.
const READ: &str = include_str!("blocked/markers.js");

/// The operation a read for the page's next call is named by, whose
/// failure no call sees.
const NEXT_CALL: &str = "page";

/// The Block cause of the document `response` answered on the page
/// `driver` drives, as `blocks` classifies it; none when no rule decides
/// one. A failure to read the document raises the code of the operation
/// `operation` that waited for its navigation; with none, for the page's
/// next call, it reads as a document holding no marker.
///
/// # Cancel safety
///
/// Cancel safe: it only reads the page.
pub(super) async fn block_of(
    driver: &PageDriver,
    blocks: &dyn BlockClassifier,
    response: &PageResponse,
    operation: Option<&'static str>,
) -> Result<Option<Block>, CallError> {
    // The Session's proxy's own answer, such as its 429, is a proxy error,
    // never a site's Block cause (ADR 0053; the proxy-or-site rule).
    if driver.proxy().is_some_and(|proxy| {
        let proxy_status = response
            .headers
            .get(proxy::PROXY_STATUS)
            .map(String::as_str);
        proxy::party(proxy.response(&response.url, proxy_status)) == proxy::Party::Proxy
    }) {
        return Ok(None);
    }
    let markers = blocks.markers(response.status);
    let holding: Vec<String> = if markers.is_empty() {
        Vec::new()
    } else {
        let read = driver
            .read_in_page(
                READ,
                json!({ "markers": markers }),
                operation.unwrap_or(NEXT_CALL),
            )
            .await;
        let found = match (read, operation) {
            (Ok(found), _) => found,
            (Err(failure), Some(_)) => return Err(failure),
            (Err(_), None) => None,
        };
        found
            .and_then(|found| serde_json::from_value(found).ok())
            .unwrap_or_default()
    };
    // The markers found, each on a line of its own, read as the document
    // reads for a classifier that looks for markers alone.
    Ok(blocks.classify(response, &holding.join("\n")))
}

/// `blocked` with `consent_required` for the consent wall the Consent
/// prompt runner left on the page `page_id`, whose events are `events`,
/// taken so it is raised once; none when it left none in the document the
/// main frame holds now (ADR 0048).
pub(super) fn consent_wall(
    walls: &Walls,
    page_id: &PageId,
    events: &PageEvents,
) -> Option<CallError> {
    let wall = walls.take(page_id)?;
    let (loader, response, received) = events.document()?;
    (loader == wall.loader).then(|| {
        blocked(
            Block {
                cause: BlockCause::ConsentRequired,
                rule: wall.rule,
            },
            &response,
            received,
        )
    })
}

/// `blocked` for `block` on the document `response` answered, received at
/// `received` on the Run's clock.
pub(super) fn blocked(block: Block, response: &PageResponse, received: SystemTime) -> CallError {
    let retry_after = response
        .headers
        .get("retry-after")
        .map(|value| {
            retry_after::read(
                value,
                response.headers.get("date").map(String::as_str),
                received,
            )
        })
        .and_then(|asked| match asked {
            RetryAfter::Until(until) => jiff::Timestamp::try_from(until).ok(),
            RetryAfter::NoWait | RetryAfter::Unparsable => None,
        });
    let asked = retry_after.map_or_else(String::new, |at| {
        format!(" The site asks for no request before {at}.")
    });
    CallError::new(
        &BLOCKED,
        format!(
            "The page `{}`, answered with {}, is a block, `{}`: {} (rule `{}`).{asked} The Session stays open with its cookies: wait, open another Session, or pause for a person.",
            response.url,
            response.status,
            block.cause.as_str(),
            block.cause.description(),
            block.rule,
        ),
        &BlockedDetails {
            cause: block.cause,
            url: response.url.clone(),
            status: Some(response.status),
            retry_after,
        },
    )
}
