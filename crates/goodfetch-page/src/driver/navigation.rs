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
//! `load` for a cut document. A gzip stream cut within a whole transfer
//! gives no signal at all: Chromium decodes its prefix and fires `load`.

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_cdp::generated::network::{self, LoaderId, RequestId, ResourceType};
use goodfetch_cdp::generated::page::{self, FrameId, FrameStartedNavigatingNavigationType};
use goodfetch_cdp::{NetError, RawEvent};
use goodfetch_engine::egress::{self, Navigator};
use goodfetch_protocol::page::{PageResponse, WaitUntil};

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
    /// Whether the main frame's `beforeunload` dialog is open.
    unloading: bool,
    /// Whether the main frame's `beforeunload` dialog was dismissed, which
    /// cancels the navigation that opened it.
    stayed: bool,
    crashed: bool,
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
            failed_committed: BTreeSet::new(),
            committed: Vec::new(),
            content_loaded: BTreeSet::new(),
            loaded: BTreeSet::new(),
            download: None,
            requested: None,
            stopped: false,
            restored: None,
            unloading: false,
            stayed: false,
            crashed: false,
        }
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
        } else if let Some(sent) = event.read::<network::RequestWillBeSentEvent>() {
            if sent.frame_id.as_ref() == Some(&self.frame)
                && sent.r#type == Some(ResourceType::Document)
            {
                // A redirect is sent again under the request's ID.
                self.chains
                    .entry(sent.loader_id.clone())
                    .or_default()
                    .push(sent.request.url);
                self.requests.insert(sent.request_id, sent.loader_id);
            }
        } else if let Some(received) = event.read::<network::ResponseReceivedEvent>() {
            if received.frame_id.as_ref() == Some(&self.frame)
                && received.r#type == ResourceType::Document
            {
                self.responses.insert(
                    received.loader_id,
                    response_of(&received.request_id, &received.response),
                );
            }
        } else if let Some(data) = event.read::<network::DataReceivedEvent>() {
            if let Some(loader) = self.requests.get(&data.request_id) {
                *self.received.entry(loader.clone()).or_default() +=
                    u64::try_from(data.data_length).unwrap_or_default();
            }
        } else if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
            if let Some(loader) = self.requests.get(&failed.request_id) {
                if self.committed.contains(loader) {
                    self.failed_committed.insert(loader.clone());
                }
                self.failed
                    .insert(loader.clone(), NetError::of(&failed.error_text));
            }
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
        } else if let Some(opening) = event.read::<page::JavascriptDialogOpeningEvent>() {
            if opening.frame_id == self.frame && opening.r#type == page::DialogType::Beforeunload {
                self.unloading = true;
            }
        } else if let Some(closed) = event.read::<page::JavascriptDialogClosedEvent>() {
            if closed.frame_id == self.frame && self.unloading {
                self.unloading = false;
                self.stayed |= !closed.result;
            }
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
            || self.stayed
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
            Some(response) if *error != NetError::Aborted && !error.fails_to_decode() => {
                Ended::Truncated {
                    received: self.received.get(loader).copied().unwrap_or_default(),
                    expected: response
                        .headers
                        .get("content-length")
                        .and_then(|length| length.trim().parse().ok()),
                }
            }
            _ => Ended::Failed(error.clone()),
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

/// The CDP response to the request `request_id` as the protocol's, its
/// header names lower-cased; a document's encoding is read from it once
/// decoded.
pub(crate) fn response_of(request_id: &RequestId, response: &network::Response) -> PageResponse {
    let headers: BTreeMap<String, String> = response
        .headers
        .iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            (name.to_ascii_lowercase(), value)
        })
        .collect();
    PageResponse {
        url: response.url.clone(),
        status: u16::try_from(response.status).unwrap_or_default(),
        status_text: response.status_text.clone(),
        headers,
        encoding: None,
        request_id: request_id.clone(),
        from_service_worker: response.from_service_worker == Some(true),
    }
}

#[cfg(test)]
mod tests;
