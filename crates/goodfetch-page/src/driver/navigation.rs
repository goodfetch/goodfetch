//! One navigation of a page's main frame, followed through the events of
//! its target (ADRs 0085, 0118, 0119 and 0121): which document it loads,
//! through which redirects, whether that document commits and reaches the
//! state the call waits for in the main frame, and its response; whether
//! the browser turned it into a download; or why it ended without one. A
//! navigation whose document another supersedes before it committed is
//! interrupted; once it committed, the call waits for the state of
//! whichever document the main frame holds then, as Playwright's does. A
//! navigation the browser failed before its document committed ends once
//! the browser's error page for it, which commits under its loader, has
//! reached that state too, or once the main frame stopped loading without
//! one: until then the browser still replaces the main frame's document,
//! and discards the DOM's node IDs at its `DOMContentLoaded`, under the
//! next call.
//!
//! The pinned Chromium's signals (ADR 0118's choice per browser): a
//! network error after the document's response arrived is a body that
//! broke off, a cut one, whatever its transport said (a length mismatch, a
//! chunked body without its last chunk, a reset or closed stream), unless
//! it is the browser's own cancel or a content coding that failed to
//! decode, which is `net::ERR_CONTENT_DECODING_FAILED`. Chromium fires no
//! `load` for a cut document. A document from the network whose load the
//! browser reported finished with no bytes counted is cut too: the network
//! counts every load it finishes, and the interception that read the body
//! at its response stage then went on before the failure arrived, which
//! it lost (ADR 0118's #1732 note); Chromium fires `load` for it. A gzip
//! stream cut within a whole transfer gives no signal at all: Chromium
//! decodes its prefix and fires `load`.

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_cdp::generated::network::{self, LoaderId, RequestId, ResourceType};
use goodfetch_cdp::generated::page::{self, FrameId, FrameStartedNavigatingNavigationType};
use goodfetch_cdp::{NetError, RawEvent, Subscription};
use goodfetch_engine::CallError;
use goodfetch_engine::egress::{self, Navigator};
use goodfetch_protocol::page::{PageResponse, WaitUntil};

use super::cdp_response::{
    cut_by, cut_though_finished, declared_length, from_the_network, response_of,
};

/// The event of a target whose renderer crashed.
pub(super) const CRASHED: &str = "Inspector.targetCrashed";

/// Whether `url` is a `javascript:` URL, which runs in a frame's document:
/// the engine's egress check classifies it (ADR 0067), as the URL Standard
/// and Chromium read it, so every spelling that check let run answers as
/// a script, not as a navigation the browser aborted.
pub(super) fn runs_a_script(url: &str) -> bool {
    // Who asked decides only where a `file:` URL may go, never a script.
    matches!(
        egress::navigation(url, Navigator::default()),
        Ok(egress::Destination::JavaScript)
    )
}

/// What `Page.navigate`'s answer, `navigated`, says the call to `url`
/// waits for: the navigation it named, or none when no document loads;
/// else how it ended. `superseded` says the engine started another
/// navigation of the page after it.
pub(super) fn navigated(
    url: &str,
    navigated: page::NavigateReturns,
    superseded: bool,
) -> Result<Option<Expected>, Ended> {
    match (navigated.error_text, navigated.loader_id) {
        // A `javascript:` URL runs in the document, which the browser
        // answers as an aborted navigation: no document loads.
        (Some(_), None) if runs_a_script(url) => Ok(None),
        (Some(error), loader) => refused(
            &NetError::of(&error),
            loader,
            navigated.is_download == Some(true),
            superseded,
        )
        .map(Some),
        (None, Some(loader)) => Ok(Some(Expected::Loader(loader))),
        // Within the document: no document loads.
        (None, None) => Ok(None),
    }
}

/// Which navigation is the call's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Expected {
    /// The one loading this document, which `Page.navigate` named.
    Loader(LoaderId),
    /// The one `Page.navigate` named, which the browser turned into a
    /// download instead of loading its document.
    Download,
    /// The first the main frame starts once the call's command is sent,
    /// which `Page.reload` and `Page.navigateToHistoryEntry` do not name.
    FirstStarted,
    /// The one `Page.navigate` named, which the browser failed with this
    /// network error before its document committed.
    Failed(LoaderId, NetError),
}

/// Where a navigation arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Arrived {
    /// At a document, whose response it holds when a server answered it.
    Document(Option<PageResponse>),
    /// At a file the browser downloads.
    Download {
        /// The file's URL.
        url: String,
        /// The name the browser suggests for it.
        suggested_filename: String,
    },
}

/// How a navigation ended without a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Ended {
    /// The browser failed it with this network error.
    Failed(NetError),
    /// Another navigation superseded it before it committed.
    Interrupted,
    /// It was redirected past the browser's limit: the URLs of its chain,
    /// the first included.
    TooManyRedirects(Vec<String>),
    /// Its document's body broke off.
    Truncated {
        /// The bytes of the body that arrived, decoded.
        received: u64,
        /// The bytes its `Content-Length` declared.
        expected: Option<u64>,
    },
    /// The page's renderer crashed.
    Crashed,
}

/// How a navigation's command and the main frame's events left it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Followed {
    /// It reached this end; `expected` is the navigation the command's
    /// answer named, when it named one.
    Ended {
        outcome: Result<Arrived, Ended>,
        expected: Option<Expected>,
    },
    /// The browser's connection ended before it did.
    Disconnected,
}

/// What the main frame's events said so far.
#[derive(Debug)]
pub(super) struct Navigation {
    frame: FrameId,
    /// Each navigation the main frame started, in order, and whether it
    /// stays within its document.
    started: Vec<(LoaderId, bool)>,
    within_document: bool,
    requests: BTreeMap<RequestId, LoaderId>,
    /// The URLs each document's request went to, through its redirects.
    chains: BTreeMap<LoaderId, Vec<String>>,
    responses: BTreeMap<LoaderId, PageResponse>,
    /// The bytes of each document's body that arrived, decoded.
    received: BTreeMap<LoaderId, u64>,
    failed: BTreeMap<LoaderId, NetError>,
    /// The documents whose response came from the network.
    from_the_network: BTreeSet<LoaderId>,
    /// The documents from the network whose load the browser reported
    /// finished with no bytes counted: their body broke off, the failure
    /// lost on its way to the page (ADR 0118's #1732 note).
    cut_though_finished: BTreeSet<LoaderId>,
    /// The documents whose navigation failed once they had committed, such
    /// as a body that broke off: the browser shows no error page for them.
    failed_committed: BTreeSet<LoaderId>,
    committed: Vec<LoaderId>,
    content_loaded: BTreeSet<LoaderId>,
    loaded: BTreeSet<LoaderId>,
    /// The download the main frame began, its URL and suggested name.
    download: Option<(String, String)>,
    /// The URL the main frame was last asked to navigate to in its own tab.
    requested: Option<String>,
    /// Whether the main frame stopped loading since the last navigation
    /// it started to another document.
    stopped: bool,
    /// The document the back/forward cache restored in the main frame,
    /// which fires no `load` and has no response: only a Borrowed browser,
    /// whose launch the engine never chose, keeps the cache on (ADR 0121).
    restored: Option<LoaderId>,
    /// What the main frame's `beforeunload` dialog asked.
    unload: Unload,
    crashed: bool,
}

/// What the main frame's `beforeunload` dialog asked since its events
/// started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unload {
    /// None closed unaccepted, nor is one open.
    Unasked,
    /// One is open.
    Asking,
    /// One was dismissed, which cancels the navigation that opened it.
    Stayed,
}

impl Navigation {
    /// A navigation of `frame`, the page's main frame, whose events start
    /// now.
    pub(super) fn of(frame: FrameId) -> Self {
        Self {
            frame,
            started: Vec::new(),
            within_document: false,
            requests: BTreeMap::new(),
            chains: BTreeMap::new(),
            responses: BTreeMap::new(),
            received: BTreeMap::new(),
            failed: BTreeMap::new(),
            from_the_network: BTreeSet::new(),
            cut_though_finished: BTreeSet::new(),
            failed_committed: BTreeSet::new(),
            committed: Vec::new(),
            content_loaded: BTreeSet::new(),
            loaded: BTreeSet::new(),
            download: None,
            requested: None,
            stopped: false,
            restored: None,
            unload: Unload::Unasked,
            crashed: false,
        }
    }

    /// Follows the navigation `command` starts through the page's
    /// `events`, read while the command runs too, until it ends or reaches
    /// the state `wait_until` names; `superseded` says the engine started
    /// another navigation of the page after it.
    ///
    /// Every event the browser sent before the command's answer is read
    /// before the answer is taken: the connection hands an event to its
    /// subscribers before it hands over a later answer, so the events are
    /// polled first (#1770). Read after the answer, the main frame's stop
    /// of the load before this navigation's, which its start then makes an
    /// earlier load's, would end a failed navigation before the browser's
    /// error page for it replaced the document.
    pub(super) async fn follow(
        &mut self,
        command: impl Future<Output = Result<Result<Option<Expected>, Ended>, CallError>>,
        events: &mut Subscription,
        wait_until: WaitUntil,
        superseded: impl Fn() -> bool,
    ) -> Result<Followed, CallError> {
        let mut command = std::pin::pin!(command);
        let mut expected = None;
        let mut reading = true;
        let outcome = loop {
            if let Some(expected) = &expected
                && let Some(outcome) = self.outcome(expected, wait_until, superseded())
            {
                break outcome;
            }
            tokio::select! {
                biased;
                event = events.next(), if reading => match event {
                    Some(event) => self.feed(&event),
                    // A target the browser detached sends nothing more, and
                    // the command in flight on it failed with it: its answer
                    // says how the navigation ended.
                    None if expected.is_none() && super::detached(events) => reading = false,
                    None => return Ok(Followed::Disconnected),
                },
                answered = &mut command, if expected.is_none() => match answered? {
                    Ok(Some(answered)) => expected = Some(answered),
                    Ok(None) => break Ok(Arrived::Document(None)),
                    Err(ended) => break Err(ended),
                },
                else => return Ok(Followed::Disconnected),
            }
        };
        Ok(Followed::Ended { outcome, expected })
    }

    /// Takes in one event of the page's target.
    pub(super) fn feed(&mut self, event: &RawEvent) {
        if event.method == CRASHED {
            self.crashed = true;
        } else if let Some(started) = event.read::<page::FrameStartedNavigatingEvent>() {
            if started.frame_id == self.frame {
                let within = matches!(
                    started.navigation_type,
                    FrameStartedNavigatingNavigationType::SameDocument
                        | FrameStartedNavigatingNavigationType::HistorySameDocument
                );
                // A stop read before belongs to an earlier load.
                if !within {
                    self.stopped = false;
                }
                self.started.push((started.loader_id, within));
            }
        } else if let Some(requested) = event.read::<page::FrameRequestedNavigationEvent>() {
            if requested.frame_id == self.frame
                && requested.disposition == page::ClientNavigationDisposition::CurrentTab
            {
                self.requested = Some(requested.url);
            }
        } else if event.method.starts_with("Network.") {
            self.feed_network(event);
        } else if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
            if navigated.frame.id == self.frame && navigated.frame.parent_id.is_none() {
                if navigated.r#type == page::NavigationType::BackForwardCacheRestore {
                    // Its requests never passed the interception or the
                    // navigation gate this time (ADR 0121).
                    tracing::info!(
                        url.origin = goodfetch_engine::log::origin(&navigated.frame.url),
                        "the browser's back/forward cache restored the page's document"
                    );
                    self.restored = Some(navigated.frame.loader_id.clone());
                }
                self.committed.push(navigated.frame.loader_id);
            }
        } else if let Some(stopped) = event.read::<page::FrameStoppedLoadingEvent>() {
            if stopped.frame_id == self.frame {
                self.stopped = true;
            }
        } else if let Some(within) = event.read::<page::NavigatedWithinDocumentEvent>() {
            if within.frame_id == self.frame {
                self.within_document = true;
            }
        } else if let Some(download) = event.read::<page::DownloadWillBeginEvent>() {
            if download.frame_id == self.frame {
                self.download = Some((download.url, download.suggested_filename));
            }
        } else if event.method.starts_with("Page.javascriptDialog") {
            self.feed_dialog(event);
        } else if let Some(lifecycle) = event.read::<page::LifecycleEventEvent>()
            && lifecycle.frame_id == self.frame
        {
            match lifecycle.name.as_str() {
                "DOMContentLoaded" => {
                    self.content_loaded.insert(lifecycle.loader_id);
                }
                "load" => {
                    self.loaded.insert(lifecycle.loader_id);
                }
                _ => {}
            }
        }
    }

    /// Takes in one network event of the page's target: the main frame's
    /// document requests, their responses, their bodies' bytes and how
    /// each load ended.
    fn feed_network(&mut self, event: &RawEvent) {
        if let Some(sent) = event.read::<network::RequestWillBeSentEvent>() {
            if sent.frame_id.as_ref() == Some(&self.frame)
                && sent.r#type == Some(ResourceType::Document)
            {
                // A redirect is sent again under the request's ID, and
                // counts a hop; any other send starts the chain anew, as
                // the page's events number a request's hops.
                let chain = self.chains.entry(sent.loader_id.clone()).or_default();
                if sent.redirect_response.is_none() {
                    chain.clear();
                }
                chain.push(sent.request.url);
                self.requests.insert(sent.request_id, sent.loader_id);
            }
        } else if let Some(received) = event.read::<network::ResponseReceivedEvent>() {
            if received.frame_id.as_ref() == Some(&self.frame)
                && received.r#type == ResourceType::Document
            {
                // The hop its redirects led to, as the page's events name it.
                let redirects = self
                    .chains
                    .get(&received.loader_id)
                    .map_or(0, |chain| chain.len().saturating_sub(1));
                if from_the_network(&received.response) {
                    self.from_the_network.insert(received.loader_id.clone());
                } else {
                    self.from_the_network.remove(&received.loader_id);
                }
                self.responses.insert(
                    received.loader_id,
                    response_of(&hop_id(&received.request_id, redirects), &received.response),
                );
            }
        } else if let Some(finished) = event.read::<network::LoadingFinishedEvent>() {
            if let Some(loader) = self.requests.get(&finished.request_id)
                && let Some(response) = self.responses.get(loader)
                && cut_though_finished(
                    None,
                    response.status,
                    self.from_the_network.contains(loader),
                    finished.encoded_data_length,
                )
            {
                self.cut_though_finished.insert(loader.clone());
            }
        } else if let Some(data) = event.read::<network::DataReceivedEvent>() {
            if let Some(loader) = self.requests.get(&data.request_id) {
                *self.received.entry(loader.clone()).or_default() +=
                    u64::try_from(data.data_length).unwrap_or_default();
            }
        } else if let Some(failed) = event.read::<network::LoadingFailedEvent>()
            && let Some(loader) = self.requests.get(&failed.request_id)
        {
            if self.committed.contains(loader) {
                self.failed_committed.insert(loader.clone());
            }
            self.failed
                .insert(loader.clone(), NetError::of(&failed.error_text));
        }
    }

    /// Takes in one dialog event of the page's target: the main frame's
    /// `beforeunload` dialog opening, and closing accepted or not.
    fn feed_dialog(&mut self, event: &RawEvent) {
        if let Some(opening) = event.read::<page::JavascriptDialogOpeningEvent>() {
            if opening.frame_id == self.frame
                && opening.r#type == page::DialogType::Beforeunload
                && self.unload == Unload::Unasked
            {
                self.unload = Unload::Asking;
            }
        } else if let Some(closed) = event.read::<page::JavascriptDialogClosedEvent>()
            && closed.frame_id == self.frame
            && self.unload == Unload::Asking
        {
            self.unload = if closed.result {
                Unload::Unasked
            } else {
                Unload::Stayed
            };
        }
    }

    /// Whether the page's renderer crashed.
    pub(super) fn crashed(&self) -> bool {
        self.crashed
    }

    /// The URL the main frame was asked to navigate to in its own tab, as a
    /// gesture's input asks it to when it follows a link or submits a form.
    pub(super) fn requested(&self) -> Option<&str> {
        self.requested.as_deref()
    }

    /// Whether the navigation the main frame was asked for reached a
    /// document that committed, or ended without one: within its document,
    /// as a download, cancelled, by the page's `beforeunload` dialog
    /// dismissed, with the main frame stopped loading once it started, or
    /// with its renderer crashed. One the browser failed otherwise ends once
    /// its error page committed under its loader. A stop read before the
    /// browser started a navigation to another document belongs to an
    /// earlier load, whatever came between: the browser's start comes after
    /// the renderer's request (#1766).
    pub(super) fn requested_ended(&self) -> bool {
        self.crashed
            || self.within_document
            || self.download.is_some()
            || self.unload == Unload::Stayed
            || (self.stopped && self.started.iter().any(|(_, within)| !within))
            || self.started.iter().any(|(loader, within)| {
                !within
                    && (self.committed.contains(loader)
                        || self
                            .failed
                            .get(loader)
                            .is_some_and(|error| *error == NetError::Aborted))
            })
    }

    /// How the `expected` navigation ended, once it did: where it arrived,
    /// once its document reached the state `wait_until` names, or why it
    /// reached no document; `superseded` says the engine started another
    /// navigation of the page after it.
    pub(super) fn outcome(
        &self,
        expected: &Expected,
        wait_until: WaitUntil,
        superseded: bool,
    ) -> Option<Result<Arrived, Ended>> {
        if self.crashed {
            return Some(Err(Ended::Crashed));
        }
        let (place, loader, refused) = match expected {
            Expected::Download => {
                return self.download.clone().map(|(url, suggested_filename)| {
                    Ok(Arrived::Download {
                        url,
                        suggested_filename,
                    })
                });
            }
            Expected::Loader(loader) => (self.place(loader), loader, None),
            Expected::Failed(loader, error) => (self.place(loader), loader, Some(error)),
            // A history navigation the back/forward cache answered: the
            // restored document arrived as it was, with no response.
            Expected::FirstStarted if self.restored.is_some() => {
                return Some(Ok(Arrived::Document(None)));
            }
            Expected::FirstStarted => {
                let (loader, within) = self.started.first()?;
                if *within {
                    return self.within_document.then_some(Ok(Arrived::Document(None)));
                }
                (Some(0), loader, None)
            }
        };
        let later: Vec<&LoaderId> = place.map_or_else(Vec::new, |place| {
            self.started[place + 1..]
                .iter()
                .map(|(started, _)| started)
                .collect()
        });
        let superseded = superseded || !later.is_empty();
        // A failure before its document committed: the browser's error page
        // for it commits under its loader, unless it was cancelled.
        let before_commit = refused.or_else(|| {
            self.failed
                .get(loader)
                .filter(|_| !self.failed_committed.contains(loader))
        });
        if let Some(error) = before_commit {
            let error_page = *error != NetError::Aborted && !superseded;
            if error_page && !self.stopped && !self.reached(loader, wait_until) {
                return None;
            }
            if *error == NetError::HttpResponseCodeFailure
                && let Some(response) = self.responses.get(loader)
            {
                return Some(Ok(Arrived::Document(Some(response.clone()))));
            }
            return Some(Err(self.ended(loader, error, superseded)));
        }
        let response = self.responses.get(loader);
        if wait_until == WaitUntil::Commit
            && self.committed.contains(loader)
            && let Some(response) = response
        {
            return Some(Ok(Arrived::Document(Some(response.clone()))));
        }
        if let Some(error) = self.failed.get(loader) {
            return Some(Err(self.ended(loader, error, superseded)));
        }
        if let Some(response) = response
            && self.cut_though_finished.contains(loader)
        {
            return Some(Err(self.truncated(loader, response)));
        }
        if !self.committed.contains(loader) {
            let overtaken = self
                .committed
                .iter()
                .any(|committed| later.contains(&committed));
            return overtaken.then_some(Err(Ended::Interrupted));
        }
        let current = self.committed.last()?;
        self.reached(current, wait_until)
            .then(|| Ok(Arrived::Document(response.cloned())))
    }

    /// Where the navigation of `loader` is among those the main frame
    /// started.
    fn place(&self, loader: &LoaderId) -> Option<usize> {
        self.started
            .iter()
            .position(|(started, _)| started == loader)
    }

    /// Whether the document of `loader` committed and reached the state
    /// `wait_until` names.
    fn reached(&self, loader: &LoaderId, wait_until: WaitUntil) -> bool {
        self.committed.contains(loader)
            && match wait_until {
                WaitUntil::Commit => true,
                WaitUntil::DomContentLoaded => self.content_loaded.contains(loader),
                WaitUntil::Load => self.loaded.contains(loader),
            }
    }

    /// Why the navigation of `loader` failed with the network `error`;
    /// `superseded` says another navigation of the page started after it.
    fn ended(&self, loader: &LoaderId, error: &NetError, superseded: bool) -> Ended {
        if *error == NetError::Aborted && superseded {
            return Ended::Interrupted;
        }
        if *error == NetError::TooManyRedirects {
            return Ended::TooManyRedirects(self.chains.get(loader).cloned().unwrap_or_default());
        }
        match self.responses.get(loader) {
            Some(response) if cut_by(error) => self.truncated(loader, response),
            _ => Ended::Failed(error.clone()),
        }
    }

    /// The document of `loader`, answered `response`, whose body broke off.
    fn truncated(&self, loader: &LoaderId, response: &PageResponse) -> Ended {
        Ended::Truncated {
            received: self.received.get(loader).copied().unwrap_or_default(),
            expected: declared_length(&response.headers),
        }
    }
}

/// Whether `error` is the browser abandoning a navigation, as it does for
/// an answer with no document and for one whose renderer crashed.
pub(super) fn abandoned(error: &NetError) -> bool {
    *error == NetError::Aborted
}

/// What a navigation `Page.navigate` refused with `error` is waited for
/// as: a download; a redirect loop, whose chain its events hold; or any
/// other failure of a navigation it gave a loader, whose error page then
/// loads, or, for a cancel, which another navigation of the page that
/// started after it may have caused. Else how it ended: `net::ERR_ABORTED`
/// is an interruption when the engine started another navigation of the
/// page after it, any other error the browser's failure of it.
pub(super) fn refused(
    error: &NetError,
    loader: Option<LoaderId>,
    download: bool,
    superseded: bool,
) -> Result<Expected, Ended> {
    if download {
        return Ok(Expected::Download);
    }
    match loader {
        Some(loader) if *error == NetError::TooManyRedirects => Ok(Expected::Loader(loader)),
        _ if *error == NetError::Aborted && superseded => Err(Ended::Interrupted),
        Some(loader) => Ok(Expected::Failed(loader, error.clone())),
        None => Err(Ended::Failed(error.clone())),
    }
}

/// The engine's ID for the hop of the browser's request `request_id` that
/// `redirects` redirects led to (ADR 0085's #703 note): the browser's ID at
/// its first hop, and that ID with [`HOP`] and the hop's number after.
pub(crate) fn hop_id(request_id: &str, redirects: usize) -> RequestId {
    if redirects == 0 {
        request_id.to_owned()
    } else {
        format!("{request_id}{HOP}{redirects}")
    }
}

/// The browser's ID for the request whose hop the engine names
/// `request_id`: its first hop's.
pub(crate) fn browser_id(request_id: &str) -> &str {
    request_id
        .split_once(HOP)
        .map_or(request_id, |(browser, _)| browser)
}

/// What sets a hop's number apart from the browser's ID in [`hop_id`],
/// which no browser's request ID holds.
const HOP: char = '#';

#[cfg(test)]
mod tests;
