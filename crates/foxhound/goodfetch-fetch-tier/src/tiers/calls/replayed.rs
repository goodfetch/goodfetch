//! `page.wait_for_response` and `page.wait_for_request_finished` on a
//! `discovered_api` page (ADRs 0020 and
//! 0093): answered from the calls the tier replays for the page's
//! document, the plan its Label learned from a sample's browser (the
//! `discovered_api` module).
//!
//! As in the browser, a wait is armed when its call arrives and answered
//! by a response that comes later: the calls the page's next navigation
//! (`goto`, `reload`, `goBack` or `goForward`) replays for the document it
//! reaches, its plan replayed one call at a time while a wait is still
//! armed. Each replayed call answers every armed wait whose URL pattern
//! matches it, and its body is kept for `response.body`. A wait no
//! replayed call answers, because the plan ended, a call failed or
//! answered no JSON, or no navigation came before its deadline, is a
//! call the tier cannot serve exactly: it escalates under `auto` (#770's
//! first build; `wait_for_request`, the
//! page's events and kept responses join with #1659).

use std::time::Duration;

use goodfetch_engine::GuardAnswer;
use goodfetch_page::url_pattern::UrlPattern;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::events::{PageExchange, PageRequest};
use goodfetch_protocol::page::request::PageRequestParams;
use goodfetch_protocol::page::route::UrlMatcher;
use goodfetch_protocol::settings::TIMEOUT_WAIT;
use serde_json::Value;
use tokio::sync::oneshot;
use url::Url;

use super::super::{FetchTiers, ServedPage, Waiter, lock};
use super::{answered, encoded, ready, sent};

/// Arms `page.wait_for_response` or `page.wait_for_request_finished`,
/// for the URLs `url` matches under `timeout`, on the served page
/// `page_id`, `page`, which replays the calls of its Label's plan.
#[expect(
    clippy::option_option,
    reason = "the param's three states as the protocol carries them (ADR 0080): absent, `null` or a duration"
)]
pub(super) fn arm(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: &ServedPage,
    operation: &'static str,
    (url, timeout): (Option<&UrlMatcher>, Option<Option<u64>>),
) -> GuardAnswer {
    let pattern = match url {
        None => UrlPattern::glob(""),
        Some(UrlMatcher::Glob { pattern }) => UrlPattern::glob(pattern),
        Some(UrlMatcher::Regex { pattern }) => UrlPattern::regex(pattern),
    };
    let pattern = match pattern {
        Ok(pattern) => pattern,
        Err(unsupported) => return ready(Err(unsupported.into())),
    };
    let (answer, answered) = oneshot::channel();
    {
        let mut pages = lock(&tiers.served.pages);
        let Some(replaying) = pages.replays.get_mut(page_id) else {
            drop(pages);
            return ready(Err(tiers.unserved(page_id, operation, true)));
        };
        replaying
            .waiters
            .retain(|waiter| !waiter.answer.is_closed());
        replaying.waiters.push(Waiter { pattern, answer });
    }
    let deadline = match timeout {
        Some(timeout) => timeout.map(Duration::from_millis),
        None => page
            .document
            .run
            .settings()
            .and_then(|settings| settings.resolved().duration(TIMEOUT_WAIT.name)),
    };
    let clock = page.document.run.clock();
    let tiers = tiers.clone();
    let page_id = page_id.clone();
    Box::pin(async move {
        let waited = async {
            match deadline {
                Some(deadline) => tokio::select! {
                    biased;
                    answer = answered => answer.ok(),
                    () = clock.sleep(deadline) => None,
                },
                None => answered.await.ok(),
            }
        };
        match waited.await {
            Some(Some(exchange)) => Ok(encoded(&exchange)),
            Some(None) | None => Err(tiers.unserved(&page_id, operation, true)),
        }
    })
}

/// Replays the calls of the plan of the served page `page_id`, `page`,
/// for the document it reached at `document`, while a wait is armed; the
/// waits none answers are told so, and escalate.
pub(super) async fn replay(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: &ServedPage,
    document: &Url,
) {
    let Some(mut replay) = lock(&tiers.served.pages)
        .replays
        .get(page_id)
        .filter(|replaying| {
            replaying
                .waiters
                .iter()
                .any(|waiter| !waiter.answer.is_closed())
        })
        .map(|replaying| replaying.plan.replay(document))
    else {
        return;
    };
    let run = page.document.run.clone();
    while armed(tiers, page_id) {
        let Some(call) = replay.next_call() else {
            break;
        };
        let params = PageRequestParams {
            page_id: page_id.clone(),
            url: call.to_string(),
            method: None,
            headers: None,
            post_data: None,
            max_redirects: Some(0),
        };
        let Ok(fetched) = sent(page, &params).await else {
            break;
        };
        let json = (200..300)
            .contains(&fetched.status)
            .then(|| serde_json::from_slice::<Value>(&fetched.body).ok())
            .flatten();
        let Some(json) = json else {
            // A call the site no longer answers as it answered the browser,
            // a signed one, a token taken once: no wait is answered from it.
            tracing::debug!(run_id = %run.id(), url = %call, status = fetched.status, "a replayed call was not answered with JSON");
            break;
        };
        let Ok(response) = answered(tiers, page_id, &run, fetched).await else {
            break;
        };
        let exchange = PageExchange {
            request: PageRequest {
                page_id: page_id.clone(),
                request_id: response.request_id.clone(),
                url: call.to_string(),
                method: "GET".to_owned(),
                headers: std::collections::BTreeMap::new(),
                post_data: None,
                resource_type: "Fetch".to_owned(),
                is_navigation_request: false,
                redirect_chain: Vec::new(),
                failure: None,
            },
            response,
        };
        answer(tiers, page_id, &exchange);
        replay.answered(&json);
    }
    // The waits left: told none came, so they escalate.
    if let Some(replaying) = lock(&tiers.served.pages).replays.get_mut(page_id) {
        for waiter in replaying.waiters.drain(..) {
            let _ = waiter.answer.send(None);
        }
    }
}

/// Whether a wait is armed on the served page `page_id`.
fn armed(tiers: &FetchTiers, page_id: &PageId) -> bool {
    lock(&tiers.served.pages)
        .replays
        .get(page_id)
        .is_some_and(|replaying| {
            replaying
                .waiters
                .iter()
                .any(|waiter| !waiter.answer.is_closed())
        })
}

/// Answers every wait armed on the served page `page_id` whose pattern
/// matches the replayed call `exchange`.
fn answer(tiers: &FetchTiers, page_id: &PageId, exchange: &PageExchange) {
    let mut pages = lock(&tiers.served.pages);
    let Some(replaying) = pages.replays.get_mut(page_id) else {
        return;
    };
    let (matched, waiting): (Vec<Waiter>, Vec<Waiter>) = replaying
        .waiters
        .drain(..)
        .partition(|waiter| waiter.pattern.matches(&exchange.request.url));
    replaying.waiters = waiting;
    for waiter in matched {
        let _ = waiter.answer.send(Some(exchange.clone()));
    }
}
