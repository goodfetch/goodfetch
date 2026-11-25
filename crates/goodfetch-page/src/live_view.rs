//! What the page crate gives the engine's Live view (ADRs 0031 and 0149):
//! each open Session's tabs, as [`WatchedTabs`] follows them, and its
//! `screencast` tier, the browser's own screencast of the shown page
//! ([`Screencast`]). Chromium sends each frame already encoded, as JPEG in
//! base64, and it goes to viewers as it came: nothing here decodes,
//! encodes or writes a frame. The tier reaches its page through one narrow
//! seam, [`ScreencastPage`], which a page's driver fills.
#![expect(
    clippy::disallowed_types,
    reason = "the tabs are read by a sync `WatchedSessions::watch` and changed under the Sessions' own lock, each lock guarding one map for one insert, removal or copy, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::generated::page::{
    ScreencastFrameAckParams, ScreencastFrameEvent, StartScreencastFormat, StartScreencastParams,
    StopScreencastParams,
};
use goodfetch_cdp::{Failure, Refusal, Subscription, SubscriptionEnd};
use goodfetch_counted_work::Row;
use goodfetch_engine::live_view::{
    FrameSize, FrameSource, SourceFrame, WatchFailure, WatchedSession, WatchedSessions, Watching,
};
use goodfetch_protocol::live_view::LiveViewTier;
use goodfetch_protocol::page::{PageId, SessionId};
use tokio::sync::watch;

use crate::driver::PageDriver;

/// The JPEG quality a screencast's frames are asked at: the one Chromium's
/// developer tools ask.
const QUALITY: i64 = 80;

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a screencast needs of its page: its events, and the screencast's
/// commands on it.
pub(crate) trait ScreencastPage: Send + Sync {
    /// The page's events from now on.
    fn events(&self) -> Subscription;

    /// Sends `command` on the page.
    ///
    /// # Errors
    ///
    /// Why the browser did not take it.
    fn send(&self, command: ScreencastCommand) -> Watching<'_, Result<(), Failure>>;
}

/// A screencast's command.
pub(crate) enum ScreencastCommand {
    /// `Page.startScreencast`.
    Start(StartScreencastParams),
    /// `Page.screencastFrameAck` of the frame this numbers.
    Acknowledge(i64),
    /// `Page.stopScreencast`.
    Stop,
}

impl ScreencastPage for PageDriver {
    fn events(&self) -> Subscription {
        PageDriver::events(self)
    }

    fn send(&self, command: ScreencastCommand) -> Watching<'_, Result<(), Failure>> {
        Box::pin(async move {
            match command {
                ScreencastCommand::Start(params) => self.command(&params).await.map(drop),
                ScreencastCommand::Acknowledge(session_id) => {
                    self.work().count(Row::LiveViewAcknowledgements);
                    self.command(&ScreencastFrameAckParams { session_id })
                        .await
                        .map(drop)
                }
                ScreencastCommand::Stop => self.command(&StopScreencastParams {}).await.map(drop),
            }
        })
    }
}

/// Each open Session's tabs, oldest first, and the page of each, which the
/// Sessions change as their pages open and close: a Session's entry leaves
/// once it closed, which ends every watch of it.
#[derive(Clone, Default)]
pub(crate) struct WatchedTabs(Arc<Mutex<BTreeMap<SessionId, Tabs>>>);

/// One open Session's tabs.
struct Tabs {
    /// Its tabs, oldest first, as its watchers read them.
    shown: watch::Sender<Vec<PageId>>,
    /// The page of each.
    pages: BTreeMap<PageId, Arc<dyn ScreencastPage>>,
}

impl WatchedTabs {
    /// The Session `session` opened, with no tab yet.
    pub(crate) fn session_opened(&self, session: &SessionId) {
        lock(&self.0)
            .entry(session.clone())
            .or_insert_with(|| Tabs {
                shown: watch::Sender::new(Vec::new()),
                pages: BTreeMap::new(),
            });
    }

    /// The Session `session` closed, or never opened: its watchers see it
    /// gone.
    pub(crate) fn session_closed(&self, session: &SessionId) {
        lock(&self.0).remove(session);
    }

    /// The Session `session` opened the tab `page`.
    pub(crate) fn page_opened(
        &self,
        session: &SessionId,
        page: &PageId,
        screencast: Arc<dyn ScreencastPage>,
    ) {
        if let Some(tabs) = lock(&self.0).get_mut(session) {
            tabs.pages.insert(page.clone(), screencast);
            tabs.shown.send_modify(|shown| shown.push(page.clone()));
        }
    }

    /// The tab `page` of the Session `session` closed, or crashed.
    pub(crate) fn page_closed(&self, session: &SessionId, page: &PageId) {
        if let Some(tabs) = lock(&self.0).get_mut(session)
            && tabs.pages.remove(page).is_some()
        {
            tabs.shown
                .send_modify(|shown| shown.retain(|open| open != page));
        }
    }

    /// The tab `page`, while the Session `session` holds it open.
    fn page(&self, session: &SessionId, page: &PageId) -> Option<Arc<dyn ScreencastPage>> {
        lock(&self.0)
            .get(session)
            .and_then(|tabs| tabs.pages.get(page))
            .cloned()
    }
}

impl WatchedSessions for WatchedTabs {
    fn watch(&self, session: &SessionId) -> Option<WatchedSession> {
        let tabs = lock(&self.0).get(session)?.shown.subscribe();
        Some(WatchedSession {
            tabs,
            sources: vec![Box::new(Screencast {
                tabs: self.clone(),
                session: session.clone(),
                capturing: None,
            })],
        })
    }
}

/// The `screencast` tier of one Session: the browser's own screencast of
/// its shown tab, one tab at a time.
struct Screencast {
    tabs: WatchedTabs,
    session: SessionId,
    capturing: Option<Capturing>,
}

/// The tab a [`Screencast`] captures.
struct Capturing {
    page: Arc<dyn ScreencastPage>,
    /// The tab's events, taken before its screencast starts so that no
    /// frame comes before them.
    events: Subscription,
    /// The frame taken and not yet acknowledged, kept so that a `next`
    /// dropped while it acknowledges loses no frame.
    taken: Option<ScreencastFrameEvent>,
}

impl FrameSource for Screencast {
    fn tier(&self) -> LiveViewTier {
        LiveViewTier::Screencast
    }

    fn start<'a>(
        &'a mut self,
        page: &'a PageId,
        size: FrameSize,
    ) -> Watching<'a, Result<(), WatchFailure>> {
        Box::pin(async move {
            let page = self
                .tabs
                .page(&self.session, page)
                .ok_or(WatchFailure::PageGone)?;
            let capturing = self.capturing.insert(Capturing {
                events: page.events(),
                page,
                taken: None,
            });
            capturing
                .page
                .send(ScreencastCommand::Start(StartScreencastParams {
                    format: Some(StartScreencastFormat::Jpeg),
                    quality: Some(QUALITY),
                    max_width: Some(i64::from(size.width)),
                    max_height: Some(i64::from(size.height)),
                    every_nth_frame: Some(1),
                    ..StartScreencastParams::default()
                }))
                .await
                .map_err(|failure| watch_failure(&failure))
        })
    }

    fn next(&mut self) -> Watching<'_, Result<SourceFrame, WatchFailure>> {
        Box::pin(async move {
            let capturing = self.capturing.as_mut().ok_or(WatchFailure::PageGone)?;
            let taken = if let Some(taken) = &capturing.taken {
                taken.session_id
            } else {
                let frame = next_frame(&mut capturing.events).await?;
                capturing.taken.insert(frame).session_id
            };
            capturing
                .page
                .send(ScreencastCommand::Acknowledge(taken))
                .await
                .map_err(|failure| watch_failure(&failure))?;
            let frame = capturing
                .taken
                .take()
                .expect("invariant: the frame acknowledged is still taken");
            Ok(SourceFrame {
                width: frame.metadata.device_width,
                height: frame.metadata.device_height,
                data: frame.data,
            })
        })
    }

    fn stop(&mut self) -> Watching<'_, ()> {
        Box::pin(async move {
            if let Some(capturing) = self.capturing.take()
                // A tab already gone stopped its screencast with it.
                && let Err(failure) = capturing.page.send(ScreencastCommand::Stop).await
            {
                tracing::debug!(%failure, "a screencast did not stop");
            }
        })
    }
}

/// The next frame among `events`.
///
/// # Errors
///
/// Why no more come: the tab or its browser is gone, or its events fell
/// behind.
async fn next_frame(events: &mut Subscription) -> Result<ScreencastFrameEvent, WatchFailure> {
    loop {
        let Some(event) = events.next().await else {
            return Err(match events.ended() {
                Some(SubscriptionEnd::Detached) => WatchFailure::PageGone,
                Some(SubscriptionEnd::FellBehind) => WatchFailure::Unrecognized(
                    "the page's events fell behind its screencast".to_owned(),
                ),
                _ => WatchFailure::BrowserGone,
            });
        };
        if let Some(frame) = event.read::<ScreencastFrameEvent>() {
            return Ok(frame);
        }
    }
}

/// The [`WatchFailure`] a failed screencast command is: where a browser's
/// failure enters the Live view.
fn watch_failure(failure: &Failure) -> WatchFailure {
    match failure {
        Failure::Refused {
            refusal: Refusal::TargetGone,
            ..
        } => WatchFailure::PageGone,
        Failure::Refused { .. } | Failure::Withheld { .. } => {
            WatchFailure::CaptureRefused(failure.to_string())
        }
        Failure::TimedOut { .. } => WatchFailure::CaptureTimedOut,
        Failure::Disconnected => WatchFailure::BrowserGone,
        _ => WatchFailure::Unrecognized(failure.to_string()),
    }
}
