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
use goodfetch_engine::events::Events;
use goodfetch_protocol::page::{
    Navigated, PAGE_CHECK_FAILED, PageId, PageResponse, SessionId, WaitUntil,
};
use serde_json::{Value, json};

use super::Pages;
use super::arrival::{arrived, judging};
use crate::blocks::{Block, BlockClassifier};
use crate::driver::{Destination, PageDriver};
use crate::interception::{self, Lists};
use crate::recovery;
use crate::sessions::{OpenPage, PageEvent};
use crate::trace;

/// The Provenance action of a page loaded with blocking off.
const BLOCKING_OFF: &str = "blocking_off";

/// A navigation's first load, judged by its Page check.
pub(super) struct FirstLoad<'a> {
    pub(super) destination: Destination<'a>,
    pub(super) wait_until: WaitUntil,
    /// Its document's response.
    pub(super) response: PageResponse,
    /// Its Page check's verdict.
    pub(super) judged: Result<(), CallError>,
    /// How many of the page's requests the Filter lists had blocked before
    /// it began.
    pub(super) blocked_before: u64,
}

impl Pages {
    /// Answers the navigation of the page `page_id` whose first load is
    /// `first`: loaded once more with blocking off when blocking may have
    /// broken it, else as its Page check judged it; a document on a domain
    /// blocking was learned off for is recorded in Provenance.
    pub(super) async fn unless_blocking_broke_it(
        &self,
        page_id: &PageId,
        page: &OpenPage,
        first: FirstLoad<'_>,
        events: &Events,
    ) -> Result<Navigated, CallError> {
        let FirstLoad {
            response,
            judged,
            blocked_before,
            ..
        } = &first;
        let loaded = |domain| Loaded {
            page_id,
            session: &page.session,
            url: &response.url,
            domain,
        };
        match self.interception.lists_for(page_id, &response.url) {
            Lists::On => {
                let touched = self.interception.lists_blocked(page_id) > *blocked_before;
                let blocks = self.blocks.as_ref();
                if let Some(reason) = reason(&page.driver, blocks, response, judged, touched).await
                {
                    return self
                        .again_without_lists(page_id, page, &first, reason, events)
                        .await;
                }
            }
            Lists::LearnedOff => {
                let domain = interception::domain_key(&response.url);
                trace::provenance(&page.run, &loaded(domain.as_deref()).learned()).await;
            }
            Lists::Off => {}
        }
        first.judged?;
        Ok(Navigated {
            response: Some(first.response),
        })
    }

    /// Loads the page `page_id` once more with its Filter lists off, since
    /// `first` failed for `reason`: to the URL its destination names, or
    /// else its document again. Blocking is learned off for the first
    /// document's domain only when this load passes; Provenance records it,
    /// and this load's outcome answers the call.
    async fn again_without_lists(
        &self,
        page_id: &PageId,
        page: &OpenPage,
        first: &FirstLoad<'_>,
        reason: Reason,
        events: &Events,
    ) -> Result<Navigated, CallError> {
        let again = match first.destination {
            Destination::Url(url) => Destination::Url(url),
            Destination::Reload | Destination::Back | Destination::Forward => Destination::Reload,
        };
        let lists_off = self.interception.lists_off(page_id);
        let navigated = page
            .life
            .call(
                PageEvent::Goto,
                page.driver.navigate(again, first.wait_until),
            )
            .await;
        if let Some(failure) = self.left_failure(page_id) {
            return Err(failure);
        }
        let response = arrived(page_id, navigated?, events).await;
        let (judged, passed) = match &response {
            Some(response) => {
                let judged = recovery::judge(&judging(page, page_id, again), response).await;
                let blocks = self.blocks.as_ref();
                let passed = passed(reason, &page.driver, blocks, response, &judged).await;
                (judged, passed)
            }
            None => (Ok(()), false),
        };
        drop(lists_off);
        let url = &first.response.url;
        let domain = if passed {
            self.interception.learn_off(page_id, url)
        } else {
            interception::domain_key(url)
        };
        let loaded = Loaded {
            page_id,
            session: &page.session,
            url,
            domain: domain.as_deref(),
        };
        trace::provenance(&page.run, &loaded.retried(reason, passed)).await;
        judged?;
        Ok(Navigated { response })
    }
}

/// Why a page is loaded once more with blocking off.
#[derive(Clone, Copy, Debug)]
enum Reason {
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
async fn reason(
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
async fn passed(
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
struct Loaded<'a> {
    page_id: &'a PageId,
    session: &'a SessionId,
    /// Its document's URL.
    url: &'a str,
    /// Its domain, as ADR 0004 keys it.
    domain: Option<&'a str>,
}

impl Loaded<'_> {
    /// The Provenance entry of the page loaded once more with blocking off
    /// for `reason`, and whether blocking was `learned` off for its domain.
    fn retried(&self, reason: Reason, learned: bool) -> Value {
        let mut entry = self.entry(reason.name());
        entry["learned"] = json!(learned);
        if let Reason::Challenge(block) = reason {
            entry["block"] = json!({ "cause": block.cause.as_str(), "rule": block.rule });
        }
        entry
    }

    /// The Provenance entry of the page loaded with blocking off since it
    /// was learned off for its domain.
    fn learned(&self) -> Value {
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
