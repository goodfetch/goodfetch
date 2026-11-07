//! `goto`, `reload`, `goBack` and `goForward` on a page a document tier
//! serves (ADR 0093): each fetches its URL over the same tier, the page's
//! Run's HTTP transport, through the navigation gate of the attempt the
//! page belongs to, its first hop and every redirect it follows each
//! judged by the Run's egress policy first, then taking its own permit
//! while that attempt's lease holds, as the crawl request's own fetch
//! follows them (ADR 0118). A refresh stub is the page its tier gives,
//! never followed: a document tier runs no refresh (ADR 0118).
//!
//! The document it reaches is judged as a browser page's is (ADRs 0086,
//! 0092 and 0093): one whose AI-use signals refuse the Run's Purpose
//! raises `site_preferences_refused` and leaves the page on a blank
//! document, and a Challenge or a captcha moves the request to the browser
//! under `auto`, its page escalated at `blocked`, or raises `blocked`
//! under a forced tier, the page left as it was. Any other document takes
//! the old one's place, and its response is the call's answer, a 404 or a
//! 500 a value. The page keeps a history of its own: `goBack` at its first
//! entry and `goForward` at its last answer no response, as the browser
//! does.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_engine::CallError;
use goodfetch_engine::runs::Run;
use goodfetch_extractor::BuiltIn;
use goodfetch_page::page_value::{DocumentResponse, NavigationGate, Permit};
use goodfetch_protocol::egress::EGRESS_REFUSED;
use goodfetch_protocol::page::{
    BLOCKED, NAVIGATION_FAILED, Navigated, NavigationFailedDetails, PageId,
};
use url::Url;

use super::super::{FetchTiers, History, Opened, ServedPage};
use super::{answered, leased, page_response, refused};
use crate::chain::{Following, Gating, HopGate, Stop, follow};
use crate::driver::{Fetched, Hop, fetch};
use crate::gate::{Gate, HandOver};
use crate::untouched::UntouchedReaders;

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
/// reached no document, `egress_refused` with its URL for a hop the Run's
/// egress policy refuses, `site_preferences_refused` for a hop the gate
/// refuses or a document whose use signals refuse the Run's Purpose,
/// `blocked` for a Challenge under a forced tier, `attempt_ended` for a
/// hop once the attempt's lease ended or a Challenge under `auto`, and the
/// chain's `too_many_redirects`, `response_unavailable` and
/// `response_truncated`.
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
    // A document tier runs no refresh: a handler's own navigation is given
    // the stub (ADR 0118).
    let following = Following {
        refresh: false,
        ..http.following.clone()
    };
    // The transport's refusal of a hop at its connection, which the chain
    // turns into a crawl request's skip: the call answers it as it is.
    let refused_at_connection = OnceLock::new();
    let on = (&clock, run.owner(), run.metrics().counted_work());
    let fetched = follow(first, &following, &gate, on, |hop| {
        // The first hop's permit is released once its answer arrived; a
        // later hop's, by the chain (ADR 0118).
        let held = permit.take();
        let refused_at_connection = &refused_at_connection;
        async move {
            let fetched = fetch(http.transport.as_ref(), &hop, &http.bounds, &clock).await;
            drop(held);
            if let Err(error) = &fetched
                && error.code() == EGRESS_REFUSED.name
            {
                let _ = refused_at_connection.set(error.clone());
            }
            fetched
        }
    })
    .await
    .map_err(|stop| match (stop, refused_at_connection.get()) {
        (Stop::Skipped(_), Some(refused)) => refused.clone(),
        (other, _) => stopped(other),
    })?;
    let extractor = page.document.extractor;
    if let Err(refusal) = Gate::new(Arc::clone(&page.hand_over))
        .judge_document(&DocumentResponse {
            url: fetched.url.as_str(),
            status: fetched.status,
            headers: &fetched.headers,
            body: &fetched.body,
        })
        .await
    {
        tiers.navigated(
            page_id,
            blank(&run, &fetched.url, (extractor, &page.hand_over.untouched)),
            at,
        );
        return Err(refused(fetched.url.as_str(), &refusal));
    }
    // A handler's own navigation takes no `fetch.encoding` and no
    // `crawl.content_types` (ADR 0119): a response that is no document
    // leaves the page holding none, and a document only a guess could read
    // is never served. The document is read on the CPU pool.
    let opened = opened_on(&run, &fetched, (extractor, &page.hand_over.untouched)).await;
    if opened.read.is_none() && opened.not_document.is_none() {
        return Err(tiers.unserved(page_id, operation, false));
    }
    if opened.not_document.is_none() {
        let reached = page_response(
            &fetched.url,
            fetched.status,
            fetched.headers.clone(),
            String::new(),
        );
        // A Challenge or a captcha a handler's own navigation meets is met
        // as the crawl request's own document's is (spec #600): under
        // `auto` the request moves to the browser, under a forced tier the
        // call fails with `blocked`.
        if let Some((_, error)) = tiers
            .classified(&run, reached, Arc::clone(&opened.document))
            .await
            .filter(|(block, _)| block.checks_the_browser())
        {
            return Err(match page.choice.forced {
                None => tiers.unserved(page_id, BLOCKED.name, false),
                Some(_) => error,
            });
        }
    }
    let mut response = answered(tiers, page_id, &run, fetched).await?;
    response.encoding = opened.read.map(|read| read.encoding.name().to_owned());
    tiers.navigated(page_id, opened, at);
    Ok(Navigated {
        response: Some(response),
    })
}

/// The document `fetched` answered a handler's own navigation of `run`,
/// read on the CPU pool, its formats with `extractor`, by `readers`'
/// readers of every document alone.
async fn opened_on(
    run: &Run,
    fetched: &Fetched,
    (extractor, readers): (Option<BuiltIn>, &Arc<UntouchedReaders>),
) -> Opened {
    let (reading, read, readers) = (run.clone(), fetched.clone(), Arc::clone(readers));
    run.owner()
        .analysis(move || Opened::of(&reading, &read, (None, extractor), (&readers, false)))
        .await
}

/// The blank document a served page is left on at `url` once the gate
/// refused the document a navigation reached there, as a browser page is
/// left on one (ADR 0093), its formats read with `extractor`.
fn blank(
    run: &Run,
    url: &Url,
    (extractor, readers): (Option<BuiltIn>, &UntouchedReaders),
) -> Opened {
    let empty = Fetched {
        url: url.clone(),
        status: 200,
        headers: vec![(
            "content-type".to_owned(),
            "text/html; charset=utf-8".to_owned(),
        )],
        body: Bytes::new(),
        unaccepted: None,
        certificate_exception: None,
    };
    Opened::of(run, &empty, (None, extractor), (readers, false))
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
            // The Run's egress policy judges the hop before anything is
            // sent for it, robots.txt included (spec #565).
            self.hand_over
                .address_check(&hop.url)
                .await
                .map_err(|refused| Stop::Failed(refused.error()))?;
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

    /// A handler's navigation records nothing in the Frontier.
    fn reached<'a>(&'a self, _hop: &'a Hop) -> Gating<'a, Result<(), Stop>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// The error a navigation's chain ended with.
fn stopped(stop: Stop) -> CallError {
    match stop {
        Stop::Failed(error) => error,
        // The navigation's gate refuses with an error, never a skip, and
        // the transport's egress refusal is answered as it is.
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
