//! `goto`, `reload`, `goBack` and `goForward` on a page a document tier
//! serves (ADR 0093): each fetches its URL over the same tier, the page's
//! Run's HTTP transport, through the navigation gate of the attempt the
//! page belongs to, its first hop and every redirect or refresh stub it
//! follows each taking its own permit while that attempt's lease holds,
//! as the crawl request's own fetch follows them (ADR 0118); the document
//! it reaches
//! takes the old one's place, and its response is the call's answer, a
//! 404 or a 500 a value. The page keeps a history of its own: `goBack` at
//! its first entry and `goForward` at its last answer no response, as the
//! browser does.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_page::page_value::Permit;
use goodfetch_protocol::page::{NAVIGATION_FAILED, Navigated, NavigationFailedDetails, PageId};
use url::Url;

use super::super::{FetchTiers, History, Opened, ServedPage};
use super::{answered, leased, refused};
use crate::chain::{Gating, HopGate, Stop, follow};
use crate::driver::{Hop, fetch};
use crate::gate::{Gate, HandOver};

/// Which navigation a call asks of a served page.
#[derive(Clone, Debug)]
pub(super) enum Step {
    /// `goto`: to this URL, a new entry of its history.
    To(String),
    /// `reload`: its entry's URL again.
    Reload,
    /// `goBack`: the entry before.
    Back,
    /// `goForward`: the entry after.
    Forward,
}

/// The navigation `step` of the served page `page_id`, `page`, answered as
/// the call `operation` answers it: a document only a guess could read is
/// a call its tier cannot serve exactly (ADR 0119).
///
/// # Errors
///
/// `navigation_failed` for a URL that does not parse or a fetch that
/// reached no document, `site_preferences_refused` for a hop the gate
/// refuses, `attempt_ended` for a hop once the attempt's lease ended, and
/// the chain's `too_many_redirects`, `response_unavailable`
/// and `response_truncated`.
pub(super) async fn navigate(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: ServedPage,
    step: Step,
    operation: &'static str,
) -> Result<Navigated, CallError> {
    let entry = |entry: usize| {
        page.history
            .get(entry)
            .map(|url| (url.clone(), History::At(entry)))
    };
    let target = match step {
        Step::To(url) => Some((url, History::Pushed)),
        Step::Reload => entry(page.at),
        Step::Back => page.at.checked_sub(1).and_then(entry),
        Step::Forward => entry(page.at + 1),
    };
    let Some((url, at)) = target else {
        return Ok(Navigated { response: None });
    };
    let url = Url::parse(&url).map_err(|error| failed(&url, &error))?;
    let run = page.document.run.clone();
    let gate = Navigating {
        hand_over: Arc::clone(&page.hand_over),
        operation,
    };
    let first = Hop::get(url);
    let mut permit = Some(gate.admit(&first).await.map_err(stopped)?);
    let clock = run.clock();
    let http = &page.http;
    let fetched = follow(first, &http.following, &gate, &clock, |hop| {
        // The first hop's permit is released once its answer arrived; a
        // later hop's, by the chain (ADR 0118).
        let held = permit.take();
        async move {
            let fetched = fetch(http.transport.as_ref(), &hop, &http.deadlines, &clock).await;
            drop(held);
            fetched
        }
    })
    .await
    .map_err(stopped)?;
    // A handler's own navigation takes no `fetch.encoding` and no
    // `crawl.content_types` (ADR 0119): a response that is no document
    // leaves the page holding none, and a document only a guess could read
    // is never served.
    let opened = Opened::of(&run, fetched.clone(), None);
    if opened.read.is_none() && opened.not_document.is_none() {
        return Err(tiers.unserved(page_id, operation, false));
    }
    let mut response = answered(tiers, page_id, &run, fetched).await?;
    response.encoding = opened.read.map(|read| read.encoding.name().to_owned());
    tiers.navigated(page_id, opened, at);
    Ok(Navigated {
        response: Some(response),
    })
}

/// The gate of a served page's navigation, the call `operation`: the
/// navigation gate of the attempt the page belongs to for each hop, the
/// first included, each sent only while that attempt's lease holds.
struct Navigating {
    hand_over: Arc<HandOver>,
    operation: &'static str,
}

impl HopGate for Navigating {
    fn admit<'a>(&'a self, hop: &'a Hop) -> Gating<'a, Result<Permit, Stop>> {
        Box::pin(async move {
            leased(&self.hand_over, self.operation).map_err(Stop::Failed)?;
            let permit = {
                let _waiting = self.hand_over.lease.permit_wait(hop.url.as_str());
                Gate::new(Arc::clone(&self.hand_over))
                    .admitted(&hop.url)
                    .await
                    .map_err(|refusal| Stop::Failed(refused(hop.url.as_str(), &refusal)))?
            };
            // The wait for the permit may outlast the lease.
            leased(&self.hand_over, self.operation).map_err(Stop::Failed)?;
            Ok(permit)
        })
    }

    fn wait_until<'a>(&'a self, hop: &'a Hop, until: SystemTime) -> Gating<'a, ()> {
        Box::pin(async move {
            let _waiting = self.hand_over.lease.permit_wait(hop.url.as_str());
            let clock = self.hand_over.run.clock();
            if let Ok(wait) = until.duration_since(clock.wall()) {
                clock.sleep(wait).await;
            }
        })
    }

    /// A handler's navigation is no crawl request's own send: it takes no
    /// token of its domain's sends, whichever hop it is.
    fn sending<'a>(&'a self, hop: &'a Hop, _lead: bool) -> Gating<'a, ()> {
        Box::pin(self.hand_over.sending(hop, false))
    }

    fn answered<'a>(&'a self, hop: &'a Hop, latency: Duration) -> Gating<'a, ()> {
        Box::pin(self.hand_over.answered(hop, latency))
    }

    fn reached<'a>(&'a self, _hop: &'a Hop) -> Gating<'a, ()> {
        Box::pin(std::future::ready(()))
    }
}

/// The error a navigation's chain ended with.
fn stopped(stop: Stop) -> CallError {
    match stop {
        Stop::Failed(error) => error,
        // The navigation's gate refuses with an error, never a skip.
        Stop::Skipped(skip) => CallError::new(
            &NAVIGATION_FAILED,
            skip.message,
            &NavigationFailedDetails {
                url: None,
                error: None,
            },
        ),
    }
}

/// `navigation_failed` for a navigation to `url` that could not start.
fn failed(url: &str, error: &impl std::fmt::Display) -> CallError {
    CallError::new(
        &NAVIGATION_FAILED,
        format!("The navigation to {url} could not start: {error}."),
        &NavigationFailedDetails {
            url: Some(url.to_owned()),
            error: None,
        },
    )
}
