//! A page loaded once more with blocking off (ADR 0047), so that a Filter
//! list that breaks a site is caught: a page with no Block cause that fails
//! its Page check while its Session's lists are on, and a challenge whose
//! requests the lists blocked, are loaded once more at once with the lists
//! off, and blocking is learned off for the page's domain only when that
//! load passes: its Page check, or the challenge cleared. The second load
//! answers the call that waited for the navigation; it is neither a Run's
//! retry nor a failure, and Provenance records that blocking was off, why,
//! and whether it was learned off. A page later loaded on a domain blocking
//! was learned off for is recorded too.

use goodfetch_engine::CallError;
use goodfetch_protocol::page::{PAGE_CHECK_FAILED, PageId, PageResponse, SessionId};
use serde_json::{Value, json};

use crate::blocks::{Block, BlockClassifier};
use crate::driver::PageDriver;

/// The Provenance action of a page loaded with blocking off.
const BLOCKING_OFF: &str = "blocking_off";

/// Why a page is loaded once more with blocking off.
#[derive(Clone, Copy, Debug)]
pub(super) enum Reason {
    /// It has no Block cause and failed its Page check.
    PageCheckFailed,
    /// It checks the browser, and the Filter lists blocked a request of its
    /// navigation.
    Challenge(Block),
}

impl Reason {
    /// Its name in Provenance.
    fn name(self) -> &'static str {
        match self {
            Self::PageCheckFailed => "page_check_failed",
            Self::Challenge(_) => "challenge",
        }
    }
}

/// Why the document `response` answered, which its Page check judged
/// `judged`, is loaded once more with blocking off, the page's Filter lists
/// being on and `touched` saying whether they blocked a request of its
/// navigation; none when it is not.
pub(super) async fn reason(
    driver: &PageDriver,
    blocks: &dyn BlockClassifier,
    response: &PageResponse,
    judged: &Result<(), CallError>,
    touched: bool,
) -> Option<Reason> {
    let failed = judged
        .as_ref()
        .is_err_and(|error| error.code() == PAGE_CHECK_FAILED.name);
    if !failed && !touched {
        return None;
    }
    match block(driver, blocks, response).await {
        Some(block) if touched && block.checks_the_browser() => Some(Reason::Challenge(block)),
        // Any other Block cause is the site's refusal, never blocking's.
        Some(_) => None,
        None => failed.then_some(Reason::PageCheckFailed),
    }
}

/// Whether the load with blocking off for `reason` passed: the document
/// `response` answered passes its Page check, as `judged` says, or no
/// longer checks the browser.
pub(super) async fn passed(
    reason: Reason,
    driver: &PageDriver,
    blocks: &dyn BlockClassifier,
    response: &PageResponse,
    judged: &Result<(), CallError>,
) -> bool {
    match reason {
        Reason::PageCheckFailed => judged.is_ok(),
        Reason::Challenge(_) => block(driver, blocks, response)
            .await
            .is_none_or(|block| !block.checks_the_browser()),
    }
}

/// The Block cause of the document `response` answered, as the page now
/// holds it; none when no rule decides one or the document is gone.
async fn block(
    driver: &PageDriver,
    blocks: &dyn BlockClassifier,
    response: &PageResponse,
) -> Option<Block> {
    let document = driver.document().await.ok()?;
    blocks.classify(response, &document.html)
}

/// The page whose document a Provenance entry names.
pub(super) struct Loaded<'a> {
    pub(super) page_id: &'a PageId,
    pub(super) session: &'a SessionId,
    /// Its document's URL.
    pub(super) url: &'a str,
    /// Its domain, as ADR 0004 keys it.
    pub(super) domain: Option<&'a str>,
}

impl Loaded<'_> {
    /// The Provenance entry of the page loaded once more with blocking off
    /// for `reason`, and whether blocking was `learned` off for its domain.
    pub(super) fn retried(&self, reason: Reason, learned: bool) -> Value {
        let mut entry = self.entry(reason.name());
        entry["learned"] = json!(learned);
        if let Reason::Challenge(block) = reason {
            entry["block"] = json!({ "cause": block.cause.as_str(), "rule": block.rule });
        }
        entry
    }

    /// The Provenance entry of the page loaded with blocking off since it
    /// was learned off for its domain.
    pub(super) fn learned(&self) -> Value {
        self.entry("learned")
    }

    fn entry(&self, reason: &str) -> Value {
        json!({
            "action": BLOCKING_OFF,
            "page_id": self.page_id,
            "session_id": self.session,
            "url": self.url,
            "domain": self.domain,
            "reason": reason,
        })
    }
}
