//! One navigation of a page's main frame, followed through the events of
//! its target (ADRs 0085, 0118, 0119 and 0121): which document it loads,
//! through which redirects, whether that document commits and reaches the
//! state the call waits for in the main frame, and its response; whether
//! the browser turned it into a download; or why it ended without one. A
//! navigation whose document another supersedes before it committed is
//! interrupted; once it committed, the call waits for the state of
//! whichever document the main frame holds then, as Playwright's does.
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

use goodfetch_cdp::RawEvent;
use goodfetch_cdp::generated::network::{self, LoaderId, RequestId, ResourceType};
use goodfetch_cdp::generated::page::{self, FrameId, FrameStartedNavigatingNavigationType};
use goodfetch_protocol::page::{PageResponse, WaitUntil};

/// The error text of a navigation the browser cancelled.
const ABORTED: &str = "net::ERR_ABORTED";

/// The error text of a navigation past the browser's 20 redirects.
const TOO_MANY_REDIRECTS: &str = "net::ERR_TOO_MANY_REDIRECTS";

/// The error text of a body whose listed content coding did not decode.
const DECODING_FAILED: &str = "net::ERR_CONTENT_DECODING_FAILED";

/// The event of a target whose renderer crashed.
const CRASHED: &str = "Inspector.targetCrashed";

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
    Failed(String),
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
    failed: BTreeMap<LoaderId, String>,
    committed: Vec<LoaderId>,
    content_loaded: BTreeSet<LoaderId>,
    loaded: BTreeSet<LoaderId>,
    /// The download the main frame began, its URL and suggested name.
    download: Option<(String, String)>,
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
            committed: Vec::new(),
            content_loaded: BTreeSet::new(),
            loaded: BTreeSet::new(),
            download: None,
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
                self.started.push((started.loader_id, within));
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
                self.responses
                    .insert(received.loader_id, response_of(&received.response));
            }
        } else if let Some(data) = event.read::<network::DataReceivedEvent>() {
            if let Some(loader) = self.requests.get(&data.request_id) {
                *self.received.entry(loader.clone()).or_default() +=
                    u64::try_from(data.data_length).unwrap_or_default();
            }
        } else if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
            if let Some(loader) = self.requests.get(&failed.request_id) {
                self.failed.insert(loader.clone(), failed.error_text);
            }
        } else if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
            if navigated.frame.id == self.frame && navigated.frame.parent_id.is_none() {
                self.committed.push(navigated.frame.loader_id);
            }
        } else if let Some(within) = event.read::<page::NavigatedWithinDocumentEvent>() {
            if within.frame_id == self.frame {
                self.within_document = true;
            }
        } else if let Some(download) = event.read::<page::DownloadWillBeginEvent>() {
            if download.frame_id == self.frame {
                self.download = Some((download.url, download.suggested_filename));
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
        let (place, loader) = match expected {
            Expected::Download => {
                return self.download.clone().map(|(url, suggested_filename)| {
                    Ok(Arrived::Download {
                        url,
                        suggested_filename,
                    })
                });
            }
            Expected::Loader(loader) => (
                self.started
                    .iter()
                    .position(|(started, _)| started == loader),
                loader,
            ),
            Expected::FirstStarted => {
                let (loader, within) = self.started.first()?;
                if *within {
                    return self.within_document.then_some(Ok(Arrived::Document(None)));
                }
                (Some(0), loader)
            }
        };
        let response = self.responses.get(loader);
        if wait_until == WaitUntil::Commit
            && self.committed.contains(loader)
            && let Some(response) = response
        {
            return Some(Ok(Arrived::Document(Some(response.clone()))));
        }
        let later: Vec<&LoaderId> = place.map_or_else(Vec::new, |place| {
            self.started[place + 1..]
                .iter()
                .map(|(started, _)| started)
                .collect()
        });
        if let Some(error) = self.failed.get(loader) {
            return Some(Err(self.ended(
                loader,
                error,
                superseded || !later.is_empty(),
            )));
        }
        if !self.committed.contains(loader) {
            let overtaken = self
                .committed
                .iter()
                .any(|committed| later.contains(&committed));
            return overtaken.then_some(Err(Ended::Interrupted));
        }
        let current = self.committed.last()?;
        let reached = match wait_until {
            WaitUntil::Commit => true,
            WaitUntil::DomContentLoaded => self.content_loaded.contains(current),
            WaitUntil::Load => self.loaded.contains(current),
        };
        reached.then(|| Ok(Arrived::Document(response.cloned())))
    }

    /// Why the navigation of `loader` failed with the network `error`;
    /// `superseded` says another navigation of the page started after it.
    fn ended(&self, loader: &LoaderId, error: &str, superseded: bool) -> Ended {
        if error == ABORTED && superseded {
            return Ended::Interrupted;
        }
        if error == TOO_MANY_REDIRECTS {
            return Ended::TooManyRedirects(self.chains.get(loader).cloned().unwrap_or_default());
        }
        match self.responses.get(loader) {
            Some(response) if error != ABORTED && error != DECODING_FAILED => Ended::Truncated {
                received: self.received.get(loader).copied().unwrap_or_default(),
                expected: response
                    .headers
                    .get("content-length")
                    .and_then(|length| length.trim().parse().ok()),
            },
            _ => Ended::Failed(error.to_owned()),
        }
    }
}

/// What a navigation `Page.navigate` refused with `error` is waited for
/// as: a download, or a redirect loop, whose chain its events hold; or how
/// it ended: `net::ERR_ABORTED` is an interruption when the engine started
/// another navigation of the page after it, any other error the browser's
/// failure of it.
pub(super) fn refused(
    error: &str,
    loader: Option<LoaderId>,
    download: bool,
    superseded: bool,
) -> Result<Expected, Ended> {
    if download {
        return Ok(Expected::Download);
    }
    match loader {
        Some(loader) if error == TOO_MANY_REDIRECTS => Ok(Expected::Loader(loader)),
        _ if error == ABORTED && superseded => Err(Ended::Interrupted),
        _ => Err(Ended::Failed(error.to_owned())),
    }
}

/// A CDP response as the protocol's, its header names lower-cased; its
/// encoding is read from its document once decoded.
fn response_of(response: &network::Response) -> PageResponse {
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
    }
}
