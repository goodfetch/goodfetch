//! One navigation of a page's main frame, followed through the events of
//! its target (ADRs 0085 and 0121): which document it loads, whether that
//! document commits and fires `load` in the main frame, and its response;
//! or why it ended without one. A navigation whose document another
//! supersedes before it committed is interrupted; once it committed, the
//! call waits for the `load` of whichever document the main frame holds
//! then, as Playwright's does.

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_cdp::RawEvent;
use goodfetch_cdp::generated::network::{self, LoaderId, RequestId, ResourceType};
use goodfetch_cdp::generated::page::{self, FrameId, FrameStartedNavigatingNavigationType};
use goodfetch_protocol::page::PageResponse;

/// The error text of a navigation the browser cancelled.
const ABORTED: &str = "net::ERR_ABORTED";

/// The event of a target whose renderer crashed.
const CRASHED: &str = "Inspector.targetCrashed";

/// Which navigation is the call's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Expected {
    /// The one loading this document, which `Page.navigate` named.
    Loader(LoaderId),
    /// The first the main frame starts once the call's command is sent,
    /// which `Page.reload` and `Page.navigateToHistoryEntry` do not name.
    FirstStarted,
}

/// How a navigation ended without a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Ended {
    /// The browser failed it with this network error.
    Failed(String),
    /// Another navigation superseded it before it committed.
    Interrupted,
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
    responses: BTreeMap<LoaderId, PageResponse>,
    failed: BTreeMap<LoaderId, String>,
    committed: Vec<LoaderId>,
    loaded: BTreeSet<LoaderId>,
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
            responses: BTreeMap::new(),
            failed: BTreeMap::new(),
            committed: Vec::new(),
            loaded: BTreeSet::new(),
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
                self.requests.insert(sent.request_id, sent.loader_id);
            }
        } else if let Some(received) = event.read::<network::ResponseReceivedEvent>() {
            if received.frame_id.as_ref() == Some(&self.frame)
                && received.r#type == ResourceType::Document
            {
                self.responses
                    .insert(received.loader_id, response_of(&received.response));
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
        } else if let Some(lifecycle) = event.read::<page::LifecycleEventEvent>()
            && lifecycle.frame_id == self.frame
            && lifecycle.name == "load"
        {
            self.loaded.insert(lifecycle.loader_id);
        }
    }

    /// How the `expected` navigation ended, once it did: its document's
    /// response, none for one within the document, or why it reached no
    /// document; `superseded` says the engine started another navigation of
    /// the page after it.
    pub(super) fn outcome(
        &self,
        expected: &Expected,
        superseded: bool,
    ) -> Option<Result<Option<PageResponse>, Ended>> {
        if self.crashed {
            return Some(Err(Ended::Crashed));
        }
        let (place, loader) = match expected {
            Expected::Loader(loader) => (
                self.started
                    .iter()
                    .position(|(started, _)| started == loader),
                loader,
            ),
            Expected::FirstStarted => {
                let (loader, within) = self.started.first()?;
                if *within {
                    return self.within_document.then_some(Ok(None));
                }
                (Some(0), loader)
            }
        };
        let later: Vec<&LoaderId> = place.map_or_else(Vec::new, |place| {
            self.started[place + 1..]
                .iter()
                .map(|(started, _)| started)
                .collect()
        });
        if let Some(error) = self.failed.get(loader) {
            if error == ABORTED && (superseded || !later.is_empty()) {
                return Some(Err(Ended::Interrupted));
            }
            return Some(Err(Ended::Failed(error.clone())));
        }
        if !self.committed.contains(loader) {
            let overtaken = self
                .committed
                .iter()
                .any(|committed| later.contains(&committed));
            return overtaken.then_some(Err(Ended::Interrupted));
        }
        let current = self.committed.last()?;
        self.loaded
            .contains(current)
            .then(|| Ok(self.responses.get(loader).cloned()))
    }
}

/// How a navigation `Page.navigate` refused ended: `net::ERR_ABORTED` is an
/// interruption when the engine started another navigation of the page
/// after it, any other error the browser's failure of it.
pub(super) fn refused(error: &str, superseded: bool) -> Ended {
    if error == ABORTED && superseded {
        Ended::Interrupted
    } else {
        Ended::Failed(error.to_owned())
    }
}

/// A CDP response as the protocol's, its header names lower-cased.
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
    }
}
