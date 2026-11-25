//! A Page's navigations (ADR 0146): each runs as a task of the Page's
//! owner, which answers its caller, so a navigation the Page's close
//! catches ends with it and raises `closed`. One goes to a URL, reloads
//! the page's document or steps in its history, and waits until its
//! document reaches the state the call names, through the primitives of
//! the CDP module; its end is then read as the site sent it, the
//! Session's proxy and egress relay told apart from the site, and its
//! document's encoding read in the guard's world, which the Page's owner
//! keeps (ADR 0119). The back/forward cache is off at launch, so going
//! back or forward always loads its document again (ADR 0121).
//!
//! A navigation takes no turn among the locator actions, so that a later
//! one supersedes it; a guard's world it leaves behind is made again in
//! the next document when the browser no longer knows it.

use goodfetch_engine::CallError;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::tls::CertificateInvalid;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::page::{
    PAGE_GO_BACK, PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageResponse, WaitUntil,
};
use tracing::Instrument as _;

use crate::cdp::{CdpPage, Going, NetError};
use crate::driver::PageDriver;
use crate::driver::navigation::{Arrived, Ended};
use crate::egress_relay::EgressRelay;
use crate::errors::{
    self, Call, interrupted, network_error, renderer_crashed, too_many_redirects, truncated,
};
use crate::page_owner::PageOwner;
use crate::page_worlds::PageWorlds;
use crate::proxy::{PROXY_STATUS, Party, party, proxy_error};

/// Where a navigation goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination<'a> {
    /// To a URL.
    Url(&'a str),
    /// To the page's document again.
    Reload,
    /// To the previous entry of the page's history.
    Back,
    /// To the next entry of the page's history.
    Forward,
}

impl Destination<'_> {
    /// The operation that navigates there, which a page's span records.
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::Url(_) => PAGE_GOTO.name,
            Self::Reload => PAGE_RELOAD.name,
            Self::Back => PAGE_GO_BACK.name,
            Self::Forward => PAGE_GO_FORWARD.name,
        }
    }

    /// The same destination, owned by the task that goes there.
    fn owned(self) -> To {
        match self {
            Self::Url(url) => To::Url(url.to_owned()),
            Self::Reload => To::Offset(0),
            Self::Back => To::Offset(-1),
            Self::Forward => To::Offset(1),
        }
    }
}

/// Where a navigation goes, as its task holds it.
enum To {
    /// To a URL.
    Url(String),
    /// To the entry this many steps away in the page's history: the
    /// current one again for 0.
    Offset(i64),
}

/// Navigates the page `page` drives, whose owner is `owner`, to
/// `destination`, and waits until its document reaches the state
/// `wait_until` names, within `deadline`, answering where it arrived: the
/// document's response, with the encoding it was decoded with once it
/// was; none for a navigation within the document, to a URL no server
/// answers, or to a history entry the page does not have; or the file the
/// browser downloads instead.
///
/// # Errors
///
/// The navigation's own, such as `navigation_failed` or
/// `deadline_exceeded`; `closed` once the Page's owner has gone, as when
/// the page closes during it.
///
/// # Cancel safety
///
/// Dropping the future leaves the navigation to go on in the browser.
pub(crate) async fn navigate(
    owner: &PageOwner,
    page: PageDriver,
    destination: Destination<'_>,
    wait_until: WaitUntil,
    deadline: Option<Deadline>,
) -> Result<Arrived, CallError> {
    let navigation = Navigating {
        page,
        worlds: owner.worlds().clone(),
        wait_until,
        deadline,
    };
    let to = destination.owned();
    // Boxed, so the futures of the calls that wait for it stay small.
    let work =
        Box::pin(async move { navigation.go(to).await }.instrument(tracing::Span::current()));
    owner.navigation(work).await
}

/// One navigation of a page, as its owner's task runs it.
struct Navigating {
    page: PageDriver,
    /// The worlds the engine keeps in the page's documents, the Page's
    /// owner's: its encoding is read in the guard's.
    worlds: PageWorlds,
    wait_until: WaitUntil,
    deadline: Option<Deadline>,
}

impl Navigating {
    /// Goes `to` its destination, and answers where it arrived.
    async fn go(self, to: To) -> Result<Arrived, CallError> {
        let cdp = CdpPage::of(&self.page);
        let going = match to {
            To::Url(url) => Going::to(&url),
            To::Offset(offset) => match cdp.history_entry(offset).await? {
                Some(going) => going,
                None => return Ok(Arrived::Document(None)),
            },
        };
        let url = going.url().to_owned();
        let call = Call::Navigation { url: url.clone() };
        // The relay's refusals from here on are this navigation's to raise.
        let relay_mark = self.page.relay().map(EgressRelay::mark);
        let mut events = cdp.navigation_events();
        let superseded = self.page.navigation_started();
        let waiting = self.page.navigation_waiting();
        let followed = self
            .page
            .within(
                self.deadline.as_ref(),
                waited_for(self.wait_until),
                // Boxed: the deadline that leaves out permit waits keeps it
                // in its own state across its sleeps.
                Box::pin(cdp.navigate(&going, &mut events, (&call, self.wait_until), &superseded)),
            )
            .await;
        // No later command waits on a subscription nobody reads.
        drop((waiting, events));
        let (followed, stopped) = self.as_the_site_sent(followed?);
        match followed {
            Ok(Ok(Arrived::Document(Some(response))))
                if let Some(failed) = self.answered_by_proxy(&url, &response) =>
            {
                Err(failed)
            }
            Ok(Ok(Arrived::Document(Some(response))))
                if self.wait_until != WaitUntil::Commit && !stopped =>
            {
                Ok(Arrived::Document(Some(PageResponse {
                    encoding: self.document_encoding(&cdp, &call).await?,
                    ..response
                })))
            }
            Ok(Ok(arrived)) => Ok(arrived),
            Ok(Err(Ended::Failed(error))) => Err(self.failed(&url, &error, relay_mark).await),
            Ok(Err(Ended::Interrupted)) => Err(interrupted(&url)),
            Ok(Err(Ended::TooManyRedirects(chain))) => Err(too_many_redirects(&url, chain)),
            Ok(Err(Ended::Truncated { received, expected })) => {
                Err(truncated(&url, received, expected))
            }
            Ok(Err(Ended::Crashed)) => Err(renderer_crashed()),
            Err(failed) => Err(failed),
        }
    }

    /// `followed`, a navigation's end, its response holding the site's own
    /// headers where the page's document reader had the browser read it
    /// otherwise (ADR 0119); and whether the reader stopped that navigation
    /// at the interception, the page then holding no document whose
    /// encoding could be read.
    fn as_the_site_sent(
        &self,
        followed: Result<Result<Arrived, Ended>, CallError>,
    ) -> (Result<Result<Arrived, Ended>, CallError>, bool) {
        match followed {
            Ok(Ok(Arrived::Document(Some(mut response)))) => {
                let site = self
                    .page
                    .reader()
                    .and_then(|reader| reader.site(&response.request_id));
                let stopped = site.as_ref().is_some_and(|site| site.stopped);
                if let Some(site) = site {
                    response.headers = site.headers;
                }
                (Ok(Ok(Arrived::Document(Some(response)))), stopped)
            }
            other => (other, false),
        }
    }

    /// The error of a navigation to `url` that failed with the browser's
    /// network error `error`, the Session's egress relay standing at
    /// `mark` as it began: the relay's refusal, else the proxy's own
    /// failure, else the site's, named as the browser names it when it
    /// connects itself, since behind the relay it sees only the relay's
    /// answer. A certificate the browser refused is `certificate_invalid`,
    /// named by the relay's own look at it where it can.
    async fn failed(&self, url: &str, error: &NetError, mark: Option<u64>) -> CallError {
        let relayed = self
            .page
            .relay()
            .and_then(|relay| relay.net_error(url, error, mark));
        let error = relayed.as_ref().unwrap_or(error);
        if let Some(refused) = self.refused_by_relay(mark, url) {
            return refused;
        }
        if let Some(failure) = errors::certificate_failure(error) {
            let examined = match self.page.relay() {
                Some(relay) => relay.examine(url).await,
                None => None,
            };
            let host = url::Url::parse(url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default();
            return examined
                .unwrap_or(CertificateInvalid { host, failure })
                .error();
        }
        self.page
            .proxy()
            .and_then(|proxy| proxy.failure(url, error))
            .unwrap_or_else(|| network_error(url, error.name()))
    }

    /// `egress_refused` when the Session's egress relay refused a
    /// connection since `mark`, as it was when the navigation to `url`
    /// began: the browser reports that refusal only as a network error.
    fn refused_by_relay(&self, mark: Option<u64>, url: &str) -> Option<CallError> {
        Some(self.page.relay()?.take_refusal(url, Some(mark?))?.error())
    }

    /// `proxy_unavailable` when `response`, the document's response to a
    /// navigation to `url`, is the Session's proxy's own, never the site's
    /// (the proxy-or-site rule).
    fn answered_by_proxy(&self, url: &str, response: &PageResponse) -> Option<CallError> {
        let proxy = self.page.proxy()?;
        let proxy_status = response.headers.get(PROXY_STATUS).map(String::as_str);
        if party(proxy.response(&response.url, proxy_status)) != Party::Proxy {
            return None;
        }
        let error = proxy_status.and_then(proxy_error).unwrap_or_default();
        Some(proxy.answered(url, &error))
    }

    /// The encoding the main frame's document was decoded with, read for
    /// `call` in the guard's world, the one the Page keeps or made and kept
    /// now; none when the frame holds no document that names one. The next
    /// gesture finds the world made.
    async fn document_encoding(
        &self,
        cdp: &CdpPage,
        call: &Call,
    ) -> Result<Option<String>, CallError> {
        // A world kept from the document before is gone: made again once.
        for _ in 0..2 {
            let context = match self.worlds.guard() {
                Some(context) => context,
                None => match cdp.guard_world(cdp.main_frame()).await {
                    Ok(context) => {
                        self.worlds.keep_guard(context);
                        context
                    }
                    Err(not_taken) if not_taken.context_gone() => continue,
                    Err(not_taken) => return Err(cdp.raised(call, &not_taken).await),
                },
            };
            match cdp.document_encoding(context).await {
                Ok(encoding) => return Ok(encoding),
                Err(not_taken) if not_taken.context_gone() => {
                    // Another command may have made the next document's.
                    self.worlds.forget_guard(context);
                }
                Err(not_taken) => return Err(cdp.raised(call, &not_taken).await),
            }
        }
        Ok(None)
    }
}

/// What a navigation that answers at `wait_until` waits for, as its
/// `deadline_exceeded` names it: the state, as the call spells it.
fn waited_for(wait_until: WaitUntil) -> WaitedFor {
    match wait_until {
        WaitUntil::Commit => WaitedFor::COMMIT,
        WaitUntil::DomContentLoaded => WaitedFor::DOM_CONTENT_LOADED,
        WaitUntil::Load => WaitedFor::LOAD,
    }
}
