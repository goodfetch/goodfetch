//! The calls a page served from a document answers itself (ADR 0093):
//! what its tier serves exactly, and the rest escalated under `auto` or
//! refused with `browser_required` under a forced tier (the `choice`
//! module's [`unserved`](crate::choice::unserved)).
//!
//! Which calls each tier serves exactly is the `served_calls` table's: a
//! call the table does not serve on the page's tier escalates or is
//! refused at once. Of those it serves, a document tier answers, besides
//! `page.formats`, which the page crate reads from the document:
//!
//! - `locator.wait_for` and `locator.wait_for_count` when the document
//!   already meets them, its CSS selector matched against the document's
//!   own markup, `visible` and `hidden` as the markup shows the element
//!   (the `markup` module); `page.wait_for_url` when the page's URL already
//!   matches; `page.wait_for_load_state` at once. A document with a
//!   declarative shadow root, whose matches the markup alone does not
//!   give, and a wait the document does not meet escalate, a static
//!   document never changing;
//! - `page.goto`, `page.reload`, `page.go_back` and `page.go_forward`,
//!   fetched over the same tier through the gate (the `navigation`
//!   module);
//! - `page.request`, its headers judged as Collie's are (ADR 0067), sent
//!   through the Session's HTTP transport, through the navigation gate of
//!   the attempt the page belongs to, each hop judged by the Run's egress
//!   policy first and each redirect it follows taking its own permit and
//!   Fetch's method, body and header rules (the `redirects` module), and
//!   never escalated; a response whose AI-use signals refuse the Run's
//!   Purpose raises `site_preferences_refused`, as a navigation's document
//!   does, and any other's body is kept for `response.body`;
//! - `page.close`, which closes the page;
//! - on `discovered_api`, `page.wait_for_response` and
//!   `page.wait_for_request_finished`, answered from the calls the page's
//!   next navigation replays (the `replayed` module).
//!
//! Any other call is one the tier cannot serve exactly. Once a call
//! escalated, every later call on the page fails as it did. A navigation
//! or a `page.request` checks the lease of the page's attempt before each
//! hop it sends, and once that lease ended is withheld with the lease
//! check's error, `attempt_ended` (ADRs 0003 and 0087).
//!
//! On a page holding no document, whose accepted response is no HTML
//! (ADR 0119), every call the table says reads, waits on or acts on the
//! document fails at once with `response_not_document`, on every tier,
//! never escalating; `response.body` reads its response's body, stored in
//! the Run's folder once asked for.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use data_encoding::BASE64;
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row as CountedRow};
use goodfetch_engine::hostile_input::check_headers;
use goodfetch_engine::runs::Run;
use goodfetch_engine::{CallError, Params};
use goodfetch_page::page_value::{DocumentResponse, NavigationGate, ServedDocument};
use goodfetch_page::url_pattern::UrlPattern;
use goodfetch_protocol::foxhound::fetch::{RESPONSE_NOT_DOCUMENT, ResponseNotDocumentDetails};
use goodfetch_protocol::page::events::{
    PAGE_WAIT_FOR_REQUEST_FINISHED, RESPONSE_BODY, ResponseBodyParams, WaitForRequestFinishedParams,
};
use goodfetch_protocol::page::request::{MAX_REDIRECTS, PAGE_REQUEST, PageRequestParams};
use goodfetch_protocol::page::route::UrlMatcher;
use goodfetch_protocol::page::session_pages::PAGE_CLOSE;
use goodfetch_protocol::page::waits::{
    Count, Counted, ElementState, LOCATOR_WAIT_FOR, LOCATOR_WAIT_FOR_COUNT, LocatorWaitForParams,
    PAGE_WAIT_FOR_LOAD_STATE, PAGE_WAIT_FOR_RESPONSE, PAGE_WAIT_FOR_URL, Reached,
    WaitForCountParams, WaitForExchangeParams, WaitForLoadStateParams, WaitForUrlParams,
};
use goodfetch_protocol::page::{
    GotoParams, NAVIGATION_FAILED, NavigationFailedDetails, PAGE_FORMATS, PAGE_GO_BACK,
    PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageId, PageResponse, RedirectLimit,
    SITE_PREFERENCES_REFUSED, SitePreferencesRefusedDetails, TOO_MANY_REDIRECTS,
    TooManyRedirectsDetails,
};
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails, SettingsLayer};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use url::Url;

use super::{FetchTiers, ServedPage, lock};
use crate::driver::{FetchFailure, Fetched, Hop, fetch};
use crate::gate::{self, Gate, HandOver};
use crate::redirects;
use crate::served_calls::{Row, Verdict, verdict};
use markup::{Shown, shown};
use navigation::{Step, navigate};

pub(crate) mod markup;
mod navigation;
mod replayed;

/// The answer to the call `operation`, its params `params`, on the served
/// page `page_id`; `None` for a page no tier serves, a call the page crate
/// answers from the document, or params that do not read, which the call's
/// handler refuses.
pub(super) fn answer(
    tiers: &FetchTiers,
    page_id: &PageId,
    operation: &'static str,
    params: Params<'_>,
) -> Option<Served> {
    let page = tiers.served_page(page_id)?;
    if let Some(content_type) = &page.not_document
        && Row::of(operation).is_some_and(Row::reads_document)
    {
        return Some(ready(Err(not_document(content_type, operation))));
    }
    if let Some(call) = page.escalated {
        drop(page);
        return Some(ready(Err(tiers.unserved(page_id, call, false))));
    }
    let forced = page.choice.forced.is_some();
    match verdict(operation, page.choice.tier, forced) {
        Some(Verdict::Serve(_)) => {}
        Some(Verdict::Escalate | Verdict::BrowserRequired) | None => {
            drop(page);
            return Some(ready(Err(tiers.unserved(page_id, operation, false))));
        }
    }
    match operation {
        name if name == PAGE_FORMATS.name => None,
        name if name == PAGE_CLOSE.name => {
            tiers.close(page_id);
            Some(ready(Ok(encoded(&serde_json::Map::new()))))
        }
        name if name == LOCATOR_WAIT_FOR.name => {
            let params: LocatorWaitForParams = read(params)?;
            let wait = match params.state.unwrap_or_default() {
                ElementState::Attached => Wait::One,
                ElementState::Detached => Wait::None,
                ElementState::Visible => Wait::Shown(Shown::Visible),
                ElementState::Hidden => Wait::Shown(Shown::Hidden),
            };
            Some(waited(
                tiers,
                page_id,
                page,
                operation,
                params.locator.selector,
                wait,
            ))
        }
        name if name == LOCATOR_WAIT_FOR_COUNT.name => {
            let params: WaitForCountParams = read(params)?;
            let wait = Wait::Count(params.count);
            Some(waited(
                tiers,
                page_id,
                page,
                operation,
                params.locator.selector,
                wait,
            ))
        }
        name if name == PAGE_WAIT_FOR_URL.name => {
            let params: WaitForUrlParams = read(params)?;
            Some(ready(url_waited(tiers, page_id, &page, operation, &params)))
        }
        // A static document reached every load state once it was read.
        name if name == PAGE_WAIT_FOR_LOAD_STATE.name => {
            let _: WaitForLoadStateParams = read(params)?;
            Some(ready(Ok(encoded(&Reached {}))))
        }
        name if [
            PAGE_GOTO.name,
            PAGE_RELOAD.name,
            PAGE_GO_BACK.name,
            PAGE_GO_FORWARD.name,
        ]
        .contains(&name) =>
        {
            navigated(tiers, page_id, page, name, params)
        }
        name if name == PAGE_REQUEST.name => {
            let params: PageRequestParams = read(params)?;
            let tiers = tiers.clone();
            let page_id = page_id.clone();
            Some(Box::pin(async move {
                let response = request(&tiers, &page_id, page, params).await?;
                Ok(encoded(&response))
            }))
        }
        name if name == RESPONSE_BODY.name => {
            let params: ResponseBodyParams = read(params)?;
            Some(body(tiers, page_id, page, operation, &params.request_id))
        }
        name if name == PAGE_WAIT_FOR_RESPONSE.name => {
            let params: WaitForExchangeParams = read(params)?;
            let waited = (params.url.as_ref(), params.timeout);
            Some(replayed::arm(tiers, page_id, &page, operation, waited))
        }
        name if name == PAGE_WAIT_FOR_REQUEST_FINISHED.name => {
            let params: WaitForRequestFinishedParams = read(params)?;
            let waited = (params.url.as_ref(), params.timeout);
            Some(replayed::arm(tiers, page_id, &page, operation, waited))
        }
        _ => {
            drop(page);
            Some(ready(Err(tiers.unserved(page_id, operation, false))))
        }
    }
}

/// `page.wait_for_url` with `params` on the served page `page_id`,
/// `page`: reached when its URL already matches, else a call its tier
/// cannot serve, a static document never navigating by itself.
fn url_waited(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: &ServedPage,
    operation: &'static str,
    params: &WaitForUrlParams,
) -> Result<Box<RawValue>, CallError> {
    let pattern = match &params.url {
        UrlMatcher::Glob { pattern } => UrlPattern::glob(pattern),
        UrlMatcher::Regex { pattern } => UrlPattern::regex(pattern),
    };
    match pattern {
        Err(unsupported) => Err(unsupported.into()),
        Ok(pattern) if pattern.matches(&page.document.url) => Ok(encoded(&Reached {})),
        Ok(_) => Err(tiers.unserved(page_id, operation, true)),
    }
}

/// The navigation `operation`, with `params`, of the served page
/// `page_id`, `page`: `None` for params that do not read.
fn navigated(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: Arc<ServedPage>,
    operation: &'static str,
    params: Params<'_>,
) -> Option<Served> {
    let step = match operation {
        name if name == PAGE_GOTO.name => Step::To(read::<GotoParams>(params)?.url),
        name if name == PAGE_RELOAD.name => Step::Reload,
        name if name == PAGE_GO_BACK.name => Step::Back,
        _ => Step::Forward,
    };
    let tiers = tiers.clone();
    let page_id = page_id.clone();
    Some(Box::pin(async move {
        let navigated = navigate(&tiers, &page_id, page, step, operation).await?;
        // A `discovered_api` page replays its document's calls for the
        // waits armed before it (the `replayed` module).
        if let Some(document) = navigated
            .response
            .as_ref()
            .and_then(|response| Url::parse(&response.url).ok())
        {
            replayed::replay(&tiers, &page_id, &document).await;
        }
        Ok(encoded(&navigated))
    }))
}

/// `response.body` of the request `request_id` on the served page
/// `page_id`, `page`: a `page.request`'s or a navigation's body, kept, or
/// its crawl request's own document's, stored once asked for.
fn body(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: Arc<ServedPage>,
    operation: &'static str,
    request_id: &str,
) -> Served {
    if let Some(body) = page.bodies.get(request_id) {
        return ready(Ok(encoded(body)));
    }
    let own = match &page.own {
        Some((response, body)) if response.request_id == request_id => Some((
            page.document.run.clone(),
            response.url.clone(),
            response.request_id.clone(),
            body.clone(),
        )),
        _ => None,
    };
    // `unserved` and `keep_body` change the page, which this call no longer
    // holds, so that it is changed without a copy.
    drop(page);
    let Some((run, url, request_id, body)) = own else {
        return ready(Err(tiers.unserved(page_id, operation, false)));
    };
    let tiers = tiers.clone();
    let page_id = page_id.clone();
    Box::pin(async move {
        let stored = run
            .store_body(body)
            .await
            .map_err(|error| failed(&url, &error))?;
        tiers.keep_body(&page_id, &request_id, stored.clone());
        Ok(encoded(&stored))
    })
}

/// The answer to the call `operation`, its params `params`, on the browser
/// page `page_id` while it holds no document, its response read at the
/// interception (ADR 0119): a call that reads, waits on or acts on the
/// document fails at once with `response_not_document`, and
/// `response.body` of its response reads the bytes kept; `None` for any
/// other call or page, which the page crate answers.
pub(super) fn on_browser(
    tiers: &FetchTiers,
    page_id: &PageId,
    operation: &'static str,
    params: Params<'_>,
) -> Option<Served> {
    let held = lock(&tiers.served.pages)
        .browser
        .get(page_id)
        .and_then(|read| read.not_document.clone())?;
    if Row::of(operation).is_some_and(Row::reads_document) {
        return Some(ready(Err(not_document(&held.content_type, operation))));
    }
    if operation != RESPONSE_BODY.name {
        return None;
    }
    let asked: ResponseBodyParams = read(params)?;
    if asked.request_id != held.request_id {
        return None;
    }
    Some(Box::pin(async move {
        let stored = held
            .run
            .store_body(held.body.clone())
            .await
            .map_err(|error| failed(&held.request_id, &error))?;
        Ok(encoded(&stored))
    }))
}

/// `response_not_document` for the call `operation` on a page whose
/// response, of the type `content_type`, holds no document (ADR 0119).
fn not_document(content_type: &str, operation: &str) -> CallError {
    CallError::new(
        &RESPONSE_NOT_DOCUMENT,
        format!(
            "`{operation}` reads the page's document, but its response is `{content_type}`, which holds none."
        ),
        &ResponseNotDocumentDetails {
            content_type: content_type.to_owned(),
            call: operation.to_owned(),
            fix: "Read the body from the handler's response (`ctx.response`); its page holds no document.".to_owned(),
        },
    )
}

/// `params` as `P`, or `None` when they do not read.
fn read<P: DeserializeOwned>(params: Params<'_>) -> Option<P> {
    params.read().ok()
}

/// A served page's answer to a call, its result encoded once or its
/// error, which [`FetchTiers`]'s guard copies into the call's success.
pub(super) type Served = Pin<Box<dyn Future<Output = Result<Box<RawValue>, CallError>> + Send>>;

/// An answer already known.
fn ready(answer: Result<Box<RawValue>, CallError>) -> Served {
    Box::pin(std::future::ready(answer))
}

/// `value` as a call's result, encoded into bytes once.
fn encoded(value: &impl Serialize) -> Box<RawValue> {
    serde_json::value::to_raw_value(value).expect("invariant: a protocol type encodes as JSON")
}

/// What a locator wait on a static document waits for.
#[derive(Clone, Copy)]
enum Wait {
    /// One element: `attached`.
    One,
    /// None: `detached`.
    None,
    /// A count of matches: `locator.wait_for_count`.
    Count(Count),
    /// The one element visible, or hidden or absent, as the markup shows
    /// it: `visible` or `hidden`.
    Shown(Shown),
}

/// The wait `wait` on the served page `page_id`, answered at once when the
/// document already meets it; any other, which a static document never
/// meets later, is a call its tier cannot serve.
fn waited(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: Arc<ServedPage>,
    operation: &'static str,
    selector: String,
    wait: Wait,
) -> Served {
    let tiers = tiers.clone();
    let page_id = page_id.clone();
    Box::pin(async move {
        let document = Arc::clone(&page.document);
        let work = page.document.run.metrics().counted_work().clone();
        let met = on_tree(&page.document, move |tree| match wait {
            Wait::Shown(state) => {
                let shown = shown((tree, &document.html), &selector, &work)?;
                let met = shown == state || (state == Shown::Hidden && shown == Shown::Absent);
                met.then(|| encoded(&Reached {}))
            }
            Wait::One | Wait::None | Wait::Count(_) => {
                let count = matches((tree, &document.html), &selector, &work)?;
                match wait {
                    Wait::One => (count == 1).then(|| encoded(&Reached {})),
                    Wait::None => (count == 0).then(|| encoded(&Reached {})),
                    Wait::Count(bounds) => (bounds.min.is_none_or(|min| count >= min)
                        && bounds.max.is_none_or(|max| count <= max))
                    .then(|| encoded(&Counted { count })),
                    Wait::Shown(_) => None,
                }
            }
        })
        .await
        .flatten();
        met.ok_or_else(|| tiers.unserved(&page_id, operation, true))
    })
}

/// What `read` reads of the untouched tree of `document`, a served page's
/// document: its kept tree, which its owner lends the read while a handler
/// reads the page (ADR 0138), else a parse of its HTML for this read
/// alone, on the CPU pool; none once the kept tree's owner is gone, as its
/// Run stops.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: a served page with no kept tree, a code-free crawl's, parsed for this read alone"
)]
pub(in crate::tiers) async fn on_tree<T, F>(document: &Arc<ServedDocument>, read: F) -> Option<T>
where
    F: FnOnce(&ParsedHtml) -> T + Send + 'static,
    T: Send + 'static,
{
    if let Some(tree) = &document.tree {
        return tree.read(read).await;
    }
    let served = Arc::clone(document);
    let work = served.run.metrics().counted_work().clone();
    Some(
        document
            .run
            .owner()
            .analysis(move || read(&work.parse_html(CountedRow::PageParses, &served.html)))
            .await,
    )
}

/// How many elements of `tree`, the tree of the markup `html`, the CSS
/// selector `selector` matches; `None` when the markup alone cannot tell,
/// the selector being one the matcher does not parse or the document
/// holding a declarative shadow root.
pub(crate) fn matches(
    (tree, html): (&ParsedHtml, &str),
    selector: &str,
    work: &CountedWork,
) -> Option<u64> {
    if html.to_ascii_lowercase().contains("shadowrootmode") {
        return None;
    }
    let matcher = work.selector(CountedRow::PageSelectorCompilations, selector)?;
    u64::try_from(tree.select_matcher(&matcher).length()).ok()
}

/// `page.request` from the served page `page_id`, sent on its tier's HTTP
/// transport through the navigation gate of the attempt the page belongs
/// to, following up to its count of redirects as Fetch does, each hop
/// through the gate and checking that attempt's lease first; its
/// response's body is kept for `response.body`.
async fn request(
    tiers: &FetchTiers,
    page_id: &PageId,
    page: Arc<ServedPage>,
    params: PageRequestParams,
) -> Result<PageResponse, CallError> {
    let run = page.document.run.clone();
    let fetched = sent(&page, &params).await?;
    // Its answer changes the page, which would copy it while this call
    // held it.
    drop(page);
    answered(tiers, page_id, &run, fetched).await
}

/// What a request `params` of the served page `page` was answered, its
/// redirects followed as `page.request` follows them, with Fetch's method,
/// body and header rules, each hop through the navigation gate of the
/// page's own attempt, and sent only while that attempt's lease holds.
///
/// # Errors
///
/// As [`request`]'s: `header_value_invalid` for a header no request may
/// send, `egress_refused` for a hop the Run's egress policy refuses,
/// `site_preferences_refused` for a hop the gate refuses or a response
/// whose AI-use signals refuse the Run's Purpose, and `attempt_ended` once
/// the attempt's lease ended.
async fn sent(page: &ServedPage, params: &PageRequestParams) -> Result<Fetched, CallError> {
    let max_redirects = params.max_redirects.unwrap_or(MAX_REDIRECTS);
    if max_redirects > MAX_REDIRECTS {
        return Err(CallError::new(
            &SETTING_INVALID,
            format!(
                "`max_redirects` is {max_redirects}, above {MAX_REDIRECTS}, the most redirects a request follows"
            ),
            &SettingInvalidDetails {
                key: "max_redirects".to_owned(),
                layer: Some(SettingsLayer::Run),
                reason: format!("{max_redirects} is above {MAX_REDIRECTS}"),
            },
        ));
    }
    let run = page.document.run.clone();
    let hand_over = &page.hand_over;
    let gate = Gate::new(Arc::clone(hand_over));
    let Asked {
        url,
        method,
        headers,
        body,
    } = asked(params)?;
    let mut hop = Hop {
        url,
        read: Hop::idempotent(&method),
        method,
        headers,
        body,
    };
    let mut chain = vec![hop.url.to_string()];
    loop {
        leased(hand_over, PAGE_REQUEST.name)?;
        // The Run's egress policy judges the hop before anything is sent
        // for it, robots.txt included (spec #565).
        hand_over
            .of_run
            .address_check(&hop.url)
            .await
            .map_err(|refused| refused.error())?;
        let permit = {
            let _waiting = hand_over.lease.permit_wait(hop.url.as_str());
            gate.admitted(&hop.url)
                .await
                .map_err(|refusal| refused(hop.url.as_str(), &gate::refusal(refusal)))?
        };
        // The wait for the permit may outlast the lease.
        leased(hand_over, PAGE_REQUEST.name)?;
        let fetched = fetch(
            page.http.transport.as_ref(),
            &hop,
            &page.http.bounds,
            &run.clock(),
        )
        .await;
        drop(permit);
        let fetched = fetched.map_err(FetchFailure::into_error)?;
        match location(&fetched) {
            // A count of 0 answers the 3xx itself.
            Some(_) if max_redirects == 0 => {}
            Some(next) => {
                if chain.len() > usize::try_from(max_redirects).unwrap_or(usize::MAX) {
                    return Err(too_many_redirects(&chain, max_redirects));
                }
                // Fetch's method rule: a 303 turns any method but `HEAD`
                // into a `GET`, and a 301 or 302 a `POST`, dropping the
                // body and its headers; `Authorization` is removed once a
                // hop goes to another origin (the `redirects` module).
                let to_get = (fetched.status == 303 && hop.method != http::Method::HEAD)
                    || (matches!(fetched.status, 301 | 302) && hop.method == http::Method::POST);
                hop = redirects::hop_to(&hop, next, to_get);
                hop.read = Hop::idempotent(&hop.method);
                chain.push(hop.url.to_string());
                continue;
            }
            None => {}
        }
        gate.judge_document(&DocumentResponse {
            url: fetched.url.as_str(),
            status: fetched.status,
            headers: &fetched.headers,
            body: &fetched.body,
        })
        .await
        .map_err(|refusal| refused(fetched.url.as_str(), &refusal))?;
        return Ok(fetched);
    }
}

/// Whether the lease of `hand_over`, the attempt a served page belongs
/// to, still holds, so the page may send its call `operation` (ADRs 0003
/// and 0087).
///
/// # Errors
///
/// The error the lease check withholds the call with once it ended:
/// `attempt_ended`.
pub(super) fn leased(hand_over: &HandOver, operation: &str) -> Result<(), CallError> {
    hand_over
        .lease
        .check()
        .map_err(|ended| hand_over.lease.withheld(ended, operation))
}

/// What a `page.request` asks for: its first hop's URL, method, headers and
/// body.
struct Asked {
    url: Url,
    method: http::Method,
    headers: Vec<(String, String)>,
    body: Option<Bytes>,
}

/// What `params` ask for.
///
/// # Errors
///
/// `navigation_failed` for a URL, a method or a body that does not read,
/// and `header_value_invalid` for a header value holding a CR, an LF or a
/// NUL, before anything is sent (ADR 0067).
fn asked(params: &PageRequestParams) -> Result<Asked, CallError> {
    let url = Url::parse(&params.url).map_err(|error| failed(&params.url, &error))?;
    check_headers(
        params
            .headers
            .iter()
            .flatten()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    )
    .map_err(|refused| refused.error())?;
    let method = params
        .method
        .as_deref()
        .map(|method| http::Method::from_bytes(method.to_ascii_uppercase().as_bytes()))
        .transpose()
        .map_err(|error| failed(&params.url, &error))?
        .unwrap_or(http::Method::GET);
    let headers = params
        .headers
        .iter()
        .flatten()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    let body = params
        .post_data
        .as_ref()
        .map(|data| BASE64.decode(data.as_bytes()).map(Bytes::from))
        .transpose()
        .map_err(|error| failed(&params.url, &error))?;
    Ok(Asked {
        url,
        method,
        headers,
        body,
    })
}

/// Where the redirect `fetched` answers leads, for a redirect status with
/// a `Location` that names an HTTP(S) URL.
fn location(fetched: &Fetched) -> Option<Url> {
    if !matches!(fetched.status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    fetched
        .headers
        .iter()
        .find(|(name, _)| name == "location")
        .and_then(|(_, to)| fetched.url.join(to).ok())
        .filter(|to| matches!(to.scheme(), "http" | "https"))
}

/// The response `fetched` a `page.request` of the served page `page_id`
/// got, as the call answers it, its body kept in `run`'s folder for
/// `response.body`.
async fn answered(
    tiers: &FetchTiers,
    page_id: &PageId,
    run: &Run,
    fetched: Fetched,
) -> Result<PageResponse, CallError> {
    let request_id = tiers.next_request(page_id);
    let stored = run
        .store_body(fetched.body)
        .await
        .map_err(|error| failed(fetched.url.as_str(), &error))?;
    tiers.keep_body(page_id, &request_id, stored);
    Ok(page_response(
        &fetched.url,
        fetched.status,
        fetched.headers,
        request_id,
    ))
}

/// The response from `url` with `status` and `headers`, whose body
/// `response.body` reads under `request_id`, as a call answers it: its
/// headers joined by name, as the browser joins them.
pub(in crate::tiers) fn page_response(
    url: &Url,
    status: u16,
    received: Vec<(String, String)>,
    request_id: String,
) -> PageResponse {
    let mut headers = BTreeMap::<String, String>::new();
    for (name, value) in received {
        headers
            .entry(name)
            .and_modify(|joined| {
                joined.push_str(", ");
                joined.push_str(&value);
            })
            .or_insert(value);
    }
    PageResponse {
        url: url.to_string(),
        status,
        status_text: http::StatusCode::from_u16(status)
            .ok()
            .and_then(|status| status.canonical_reason())
            .unwrap_or_default()
            .to_owned(),
        headers,
        encoding: None,
        request_id,
        from_service_worker: false,
    }
}

/// `navigation_failed` for a request to `url` that could not be sent.
fn failed(url: &str, error: &impl std::fmt::Display) -> CallError {
    CallError::new(
        &NAVIGATION_FAILED,
        format!("The request to {url} could not be sent: {error}."),
        &NavigationFailedDetails {
            url: Some(url.to_owned()),
            error: None,
        },
    )
}

/// `site_preferences_refused` for the request to `url` the gate refused, or
/// `response_limit_exceeded` when a file of the preferences passed a limit.
fn refused(url: &str, refusal: &goodfetch_page::page_value::Refusal) -> CallError {
    use goodfetch_page::page_value::Refusal;
    match refusal {
        Refusal::Preference { reason, message } => CallError::new(
            &SITE_PREFERENCES_REFUSED,
            format!("the site's preferences refused {url}: {message}"),
            &SitePreferencesRefusedDetails {
                url: url.to_owned(),
                reason: *reason,
            },
        ),
        Refusal::PastALimit(limit) => limit.error(),
    }
}

/// `too_many_redirects` for `chain`, past `max_redirects`.
fn too_many_redirects(chain: &[String], max_redirects: u32) -> CallError {
    let first = chain.first().cloned().unwrap_or_default();
    CallError::new(
        &TOO_MANY_REDIRECTS,
        format!(
            "the request to {first} was redirected again after {max_redirects} redirects, past its `max_redirects`"
        ),
        &TooManyRedirectsDetails {
            url: first.clone(),
            hops: chain.iter().skip(1).cloned().collect(),
            limit: RedirectLimit::Option {
                name: "max_redirects".to_owned(),
                count: max_redirects,
            },
        },
    )
}

#[cfg(test)]
mod tests;
