//! The fetch suite's cases (spec #579; ADRs 0020 and 0093), which need a
//! crawl and so run on Foxhound's executable (ADR 0094): a crawl request
//! under `fetch.tier: http` is fetched by the engine over HTTP, its content
//! coding decoded, and its handler, the harness, is handed a page served
//! from the document, which `page.formats` reads; the site sees one
//! request for the page, beside the robots.txt the crawl reads (ADR 0086).
//! The harness names the crawl's operations by their wire names, since
//! only `goodfetch-protocol` names its Foxhound module (ADR 0099).
//! The page is the `content-coding` family's gzip page, over plain HTTP.
//! The fetch edge cases of the `http` tier (ADR 0118) are `edge`'s, and
//! those of its redirect chains `chain`'s.

use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::PAGE_FORMATS;
use goodfetch_protocol::run::RUN_FINISH;
use serde_json::{Value, json};

use super::{Checks, Site};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::DEADLINE;

mod chain;
mod edge;

/// `crawl.start`, Foxhound's.
const CRAWL_START: &str = "crawl.start";

/// `crawl_request.handle`, Foxhound's handler request.
const CRAWL_REQUEST_HANDLE: &str = "crawl_request.handle";

/// The task whose page the crawl fetches: labelled `gzip`.
const TASK: &str = "content-coding/8";

/// The text the page holds once decoded.
const TEXT: &str = "Content coding page 8, decoded once and as labelled.";

/// A case of the fetch suite, by its ID in its case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    HttpTierHandsTheDocumentOnAPage,
    CutBodiesFailTheirAttempt,
    CloseDelimitedBodyIsWhole,
    UnprocessedRequestIsResent,
    RedirectLoopCostsItsCountAndOne,
    RedirectsSendTheFetchMethods,
    OtherRedirectsAreNotOk,
    HopsGoThroughTheGate,
    HopsGoThroughTheFrontier,
    RefusedHopsEndSkipped,
    RedirectRetryAfter,
    HeldPastTheBoundAreRetried,
    RefreshStubs,
}

impl Case {
    /// The case `id` names, if it is one of the fetch suite's.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        match id {
            "a-request-fetched-over-http-hands-its-handler-the-document-on-a-page" => {
                Some(Self::HttpTierHandsTheDocumentOnAPage)
            }
            "a-cut-body-fails-its-attempt-with-response-truncated-and-never-reaches-the-handler" => {
                Some(Self::CutBodiesFailTheirAttempt)
            }
            "a-body-its-connections-close-delimits-reaches-the-handler-whole" => {
                Some(Self::CloseDelimitedBodyIsWhole)
            }
            "a-request-the-site-never-processed-is-sent-once-more-and-counted-nowhere" => {
                Some(Self::UnprocessedRequestIsResent)
            }
            "a-redirect-loop-fails-with-too-many-redirects-at-a-cost-of-fetch-redirect-max-and-one-requests" => {
                Some(Self::RedirectLoopCostsItsCountAndOne)
            }
            "each-redirect-status-sends-its-next-hop-with-the-method-and-body-fetch-specifies" => {
                Some(Self::RedirectsSendTheFetchMethods)
            }
            "a-3xx-fetch-does-not-follow-fails-with-response-not-ok-unless-crawl-handle-statuses-lists-it" => {
                Some(Self::OtherRedirectsAreNotOk)
            }
            "each-redirect-hop-goes-through-the-navigation-gate" => {
                Some(Self::HopsGoThroughTheGate)
            }
            "a-redirect-to-an-admitted-key-or-out-of-scope-ends-skipped" => {
                Some(Self::HopsGoThroughTheFrontier)
            }
            "a-redirect-the-scope-the-domain-policy-or-the-address-check-refuses-ends-skipped-live" => {
                Some(Self::RefusedHopsEndSkipped)
            }
            "a-redirects-retry-after-delays-its-next-hop-or-past-the-bound-fails-with-response-unavailable" => {
                Some(Self::RedirectRetryAfter)
            }
            "requests-held-past-politeness-retry-after-max-end-failed-and-retry-failed-re-admits-them" => {
                Some(Self::HeldPastTheBoundAreRetried)
            }
            "a-refresh-stub-within-fetch-refresh-max-delay-hands-the-handler-its-target" => {
                Some(Self::RefreshStubs)
            }
            _ => None,
        }
    }

    /// Runs the case on `connection` against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::HttpTierHandsTheDocumentOnAPage => http_tier(connection, site, checks),
            Self::CutBodiesFailTheirAttempt => edge::cut_bodies(connection, site, checks),
            Self::CloseDelimitedBodyIsWhole => edge::close_delimited(connection, site, checks),
            Self::UnprocessedRequestIsResent => edge::resent(connection, site, checks),
            Self::RedirectLoopCostsItsCountAndOne => chain::redirect_loop(connection, site, checks),
            Self::RedirectsSendTheFetchMethods => chain::redirect_methods(connection, site, checks),
            Self::OtherRedirectsAreNotOk => chain::not_followed(connection, site, checks),
            Self::HopsGoThroughTheGate => chain::gated_hops(connection, site, checks),
            Self::HopsGoThroughTheFrontier => chain::frontier_hops(connection, site, checks),
            Self::RefusedHopsEndSkipped => chain::refused_hops(connection, site, checks),
            Self::RedirectRetryAfter => chain::redirect_retry_after(connection, site, checks),
            Self::HeldPastTheBoundAreRetried => edge::held_past_the_bound(connection, site, checks),
            Self::RefreshStubs => chain::refresh_stubs(connection, site, checks),
        }
    }
}

/// The page's plain HTTP URL.
fn page_url(site: &Site) -> Option<String> {
    site.start_url(TASK)
        .map(|url| url.replacen("https://", "http://", 1))
}

/// The request target `url` names: its path and query.
fn target_of(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    rest.find('/').map(|slash| &rest[slash..])
}

/// A crawl of the page under `fetch.tier: http`: the handler request names
/// the tier and a page whose Markdown holds the page's decoded text; the
/// Proving ground saw one request for it. The site's robots.txt, which the
/// crawl also reads (ADR 0086), is not a request for the page.
fn http_tier(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-fetch-tier-http";
    let url = page_url(site);
    checks.check(
        format!("the Proving ground serves task `{TASK}`"),
        url.is_some(),
    );
    let Some(url) = url else {
        return Ok(());
    };
    let settings = site.mapped(
        &json!({ "fetch": { "tier": "http" } }),
        connection.acts_for_model(),
    );
    let id = connection.request(
        CRAWL_START,
        &json!({
            "run_id": run_id,
            "seeds": [{ "url": url, "label": "page" }],
            "settings": settings,
        }),
    )?;
    let answer = connection.answer(id)?;
    checks.check(
        "`crawl.start` starts a new Run",
        answer.message["result"]["started_as"] == json!("new"),
    );
    let mut queued = answer.others.into_iter();
    let handle = match queued.next() {
        Some(message) => message,
        None => connection.next_within(DEADLINE)?,
    };
    let params = &handle["params"];
    checks.check(
        "the handler is asked to handle the request",
        handle["method"] == json!(CRAWL_REQUEST_HANDLE)
            && params["crawl_request"]["url"] == json!(url),
    );
    checks.check(
        "the handler request names the `http` tier",
        params["tier"] == json!("http"),
    );
    let page_id = params["page_id"].clone();
    checks.check(
        "the handler request names the page served from the document",
        page_id.is_string(),
    );
    let read = connection.request(
        PAGE_FORMATS.name,
        &json!({ "page_id": page_id, "formats": { "markdown": true } }),
    )?;
    let formats = connection.answer(read)?.message;
    let markdown = formats["result"]["markdown"].as_str().unwrap_or_default();
    checks.check(
        "`page.formats` reads the page's decoded document",
        formats["result"]["url"] == json!(url) && markdown.contains(TEXT),
    );
    connection.reply(&handle["id"], &json!({ "records": [] }))?;
    let finish = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    let finished = connection.answer(finish)?.message;
    checks.check(
        "the Run finishes",
        finished["result"]["lifecycle"] == json!("finished"),
    );
    let requests = site.received_requests();
    let target = target_of(&url);
    let seen = requests
        .of_the_case()
        .filter(|request| {
            request.task.as_deref() == Some(TASK) && Some(request.target.as_str()) == target
        })
        .count();
    checks.check("the site saw one request for the page", seen == 1);
    let closed = connection.request(
        PAGE_FORMATS.name,
        &json!({ "page_id": page_id, "formats": { "markdown": true } }),
    )?;
    let closed: Value = connection.answer(closed)?.message;
    checks.check(
        "the page is closed once its attempt ended",
        closed["error"]["data"]["code"] == CLOSED.name,
    );
    Ok(())
}
