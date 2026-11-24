//! A page loaded once more with blocking off (ADR 0047), so that a Filter
//! list that breaks a site is caught: a page with no Block cause that fails
//! its Page check while its Session's lists are on, and a challenge whose
//! requests the lists blocked, are loaded once more at once with the lists
//! off, and blocking is learned off for the page's domain only when that
//! load passes: its Page check, or the challenge cleared. The second load
//! answers the call that waited for the navigation; it is neither a Run's
//! retry nor a failure, and Provenance records that blocking was off, why,
//! and whether it was learned off, a load that failed included. The first
//! load's Block cause is the one its navigation judged, and the second's is
//! judged the same way, so no document is read twice for it. A page later
//! loaded on a domain blocking was learned off for is recorded too.

use goodfetch_counted_work::Row;
use goodfetch_engine::CallError;
use goodfetch_engine::events::Events;
use goodfetch_protocol::page::PageResponse;
use goodfetch_protocol::page::{Navigated, PAGE_CHECK_FAILED, PageId, SessionId, WaitUntil};
use serde_json::{Value, json};

use super::Pages;
use super::arrival::{arrived, judging};
use super::blocked::{block_of, blocked};
use crate::blocks::Block;
use crate::driver::Destination;
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
    /// Its call's own `timeout` param (ADR 0080), which the load again
    /// waits under too.
    #[expect(
        clippy::option_option,
        reason = "the param's three states as the protocol carries them (ADR 0080): absent, `null` or a duration"
    )]
    pub(super) timeout: Option<Option<u64>>,
    /// Its document's response.
    pub(super) response: PageResponse,
    /// Its Page check's verdict.
    pub(super) judged: Result<(), CallError>,
    /// Its document's Block cause, as its navigation judged it.
    pub(super) block: Option<Block>,
    /// Whether the Filter lists blocked a request of its navigation.
    pub(super) touched: bool,
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
            block,
            touched,
            ..
        } = &first;
        let loaded = |domain| Loaded {
            page_id,
            session: &page.session,
            url: &response.url,
            domain,
        };
        match page.owner.interception().lists_for(&response.url) {
            Lists::On => {
                if let Some(reason) = reason(block.as_ref(), judged, *touched) {
                    return self
                        .again_without_lists(page_id, page, &first, reason, events)
                        .await;
                }
            }
            Lists::LearnedOff => {
                let domain = interception::domain_key(&response.url);
                trace::counted_provenance(
                    &page.run,
                    Row::NavigationProvenanceEntries,
                    loaded(domain.as_deref()).learned(),
                )
                .await;
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
    /// a load that failed included, and this load's outcome answers the
    /// call: its document's Block cause, judged as the first's was, raises
    /// `blocked`.
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
        let deadline = page.driver.navigation_deadline(first.timeout)?;
        let lists_off = page.owner.interception().lists_off();
        let navigated = page
            .life
            .call(
                PageEvent::Goto,
                page.driver
                    .navigate(again, first.wait_until, deadline.as_ref()),
            )
            .await;
        let navigated = match Self::left_failure(page) {
            Some(failure) => Err(failure),
            None => navigated,
        };
        let loaded = match navigated {
            Ok(navigated) => {
                self.judged_again(page_id, page, again, navigated, events)
                    .await
            }
            Err(failure) => Err(failure),
        };
        drop(lists_off);
        let passed = match &loaded {
            Ok(Some((_, judged, block))) => passed(&reason, judged, block.as_ref()),
            _ => false,
        };
        self.retried(page_id, page, &first.response.url, reason, passed)
            .await;
        let Some((response, judged, block)) = loaded? else {
            return Ok(Navigated { response: None });
        };
        judged?;
        if let Some(block) = block {
            let received = self
                .events(page_id)?
                .answered_at(&response.request_id)
                .unwrap_or_else(|| page.clock.wall());
            return Err(blocked(block, &response, received));
        }
        Ok(Navigated {
            response: Some(response),
        })
    }

    /// The second load's document, `navigated` to `again`: its response,
    /// its Page check's verdict and its Block cause, judged as the first
    /// load's were; none when no document arrived.
    async fn judged_again(
        &self,
        page_id: &PageId,
        page: &OpenPage,
        again: Destination<'_>,
        navigated: crate::driver::Arrived,
        events: &Events,
    ) -> Result<Option<(PageResponse, Result<(), CallError>, Option<Block>)>, CallError> {
        let Some(response) = arrived(page_id, navigated, events).await else {
            return Ok(None);
        };
        self.events(page_id)?.judged(&response.request_id);
        let block = block_of(
            &page.driver,
            page.run.metrics().counted_work(),
            self.blocks.as_ref(),
            &response,
            Some(again.operation()),
        )
        .await?;
        let judged = recovery::judge(&judging(page, page_id, again), &response).await;
        Ok(Some((response, judged, block)))
    }

    /// Records in Provenance that the page `page_id`'s document at `url`
    /// was loaded once more with blocking off for `reason`, and whether that
    /// load `passed`, which learns blocking off for its domain.
    async fn retried(
        &self,
        page_id: &PageId,
        page: &OpenPage,
        url: &str,
        reason: Reason,
        passed: bool,
    ) {
        let domain = if passed {
            page.owner.interception().learn_off(url)
        } else {
            interception::domain_key(url)
        };
        let loaded = Loaded {
            page_id,
            session: &page.session,
            url,
            domain: domain.as_deref(),
        };
        trace::counted_provenance(
            &page.run,
            Row::NavigationProvenanceEntries,
            loaded.retried(&reason, passed),
        )
        .await;
    }
}

/// Why a page is loaded once more with blocking off.
#[derive(Clone, Debug)]
enum Reason {
    /// It has no Block cause and failed its Page check.
    PageCheckFailed,
    /// It checks the browser, and the Filter lists blocked a request of its
    /// navigation.
    Challenge(Block),
}

impl Reason {
    /// Its name in Provenance.
    fn name(&self) -> &'static str {
        match self {
            Self::PageCheckFailed => "page_check_failed",
            Self::Challenge(_) => "challenge",
        }
    }
}

/// Why a document whose Block cause is `block`, which its Page check judged
/// `judged`, is loaded once more with blocking off, the page's Filter lists
/// being on and `touched` saying whether they blocked a request of its
/// navigation; none when it is not.
fn reason(block: Option<&Block>, judged: &Result<(), CallError>, touched: bool) -> Option<Reason> {
    let failed = judged
        .as_ref()
        .is_err_and(|error| error.code() == PAGE_CHECK_FAILED.name);
    if !failed && !touched {
        return None;
    }
    match block {
        Some(block) if touched && block.checks_the_browser() => {
            Some(Reason::Challenge(block.clone()))
        }
        // Any other Block cause is the site's refusal, never blocking's.
        Some(_) => None,
        None => failed.then_some(Reason::PageCheckFailed),
    }
}

/// Whether the load with blocking off for `reason` passed: its document
/// passes its Page check, as `judged` says, or, its Block cause being
/// `block`, no longer checks the browser.
fn passed(reason: &Reason, judged: &Result<(), CallError>, block: Option<&Block>) -> bool {
    match reason {
        Reason::PageCheckFailed => judged.is_ok(),
        Reason::Challenge(_) => block.is_none_or(|block| !block.checks_the_browser()),
    }
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
    fn retried(&self, reason: &Reason, learned: bool) -> Value {
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
