//! The driver: one task, the feed, per Session somebody watches, which
//! captures it by its best tier that works and publishes the newest frame;
//! each viewer's [`Viewing`] takes that frame once it acknowledged the one
//! before, so a slow viewer holds up neither the capture nor another
//! viewer, and no frame waits behind another.

#![expect(
    clippy::disallowed_types,
    reason = "which Sessions are watched is read or changed in one step, never across an `.await`, and whoever offers Sessions never calls back in, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use goodfetch_protocol::live_view::{
    LiveViewEnd, LiveViewEngineMessage, LiveViewScreen, LiveViewTab, LiveViewTier,
};
use goodfetch_protocol::page::{PageId, SessionId};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Utf8Bytes;

use super::{FrameSize, FrameSource, SourceFrame, VideoOutput, WatchFailure, WatchedSessions};
use crate::clock::Clock;
use crate::tasks::{OwnerKind, TaskOwner};

/// How long a tier has to start, and a stopped one to say it stopped.
const START_WITHIN: Duration = Duration::from_secs(10);

/// The screens that watch one Session, by viewer.
type Viewers = BTreeMap<u64, LiveViewScreen>;

/// A Daemon's Live view; clones are the same one.
#[derive(Clone)]
pub(crate) struct LiveView(Arc<Inner>);

struct Inner {
    sessions: Arc<[Arc<dyn WatchedSessions>]>,
    owner: TaskOwner,
    clock: Clock,
    watches: Mutex<Watches>,
}

#[derive(Default)]
struct Watches {
    open: BTreeMap<SessionId, Watch>,
    /// How many viewers and feeds there were, which numbers the next.
    numbered: u64,
}

impl Watches {
    fn next_number(&mut self) -> u64 {
        self.numbered += 1;
        self.numbered
    }
}

/// A watched Session: its feed's number, its viewers and what it shows.
struct Watch {
    feed: u64,
    viewers: Arc<watch::Sender<Viewers>>,
    showing: watch::Receiver<Showing>,
}

/// Why a viewer was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NotWatched {
    /// No open Session has the ID.
    UnknownSession,
    /// As many viewers as the limit already watch it.
    ViewerLimit,
    /// None of the Session's tiers is one `live_view.tier` takes.
    NoTier,
}

/// The tiers a Session may be captured by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tiers {
    /// The best that works.
    Any,
    /// This one only.
    Only(LiveViewTier),
}

impl Tiers {
    fn take(self, tier: LiveViewTier) -> bool {
        match self {
            Self::Any => true,
            Self::Only(only) => only == tier,
        }
    }
}

/// A frame as its viewers get it, encoded once for all of them, when the
/// first one takes it.
#[derive(Debug)]
pub(super) struct Frame {
    pub(super) number: u64,
    page: PageId,
    message: LiveViewEngineMessage,
    text: OnceLock<Utf8Bytes>,
}

impl Frame {
    fn new(number: u64, page: PageId, frame: SourceFrame) -> Self {
        let message = LiveViewEngineMessage::Frame {
            frame: number,
            page: page.clone(),
            width: frame.width,
            height: frame.height,
            data: frame.data,
        };
        Self {
            number,
            page,
            message,
            text: OnceLock::new(),
        }
    }

    /// Its message's text.
    pub(super) fn text(&self) -> Utf8Bytes {
        self.text
            .get_or_init(|| {
                serde_json::to_string(&self.message)
                    .expect("invariant: a frame encodes")
                    .into()
            })
            .clone()
    }
}

/// What a watched Session shows now.
struct Showing {
    tier: LiveViewTier,
    tabs: Arc<[PageId]>,
    shown: Option<PageId>,
    frame: Option<Arc<Frame>>,
    ended: Option<LiveViewEnd>,
}

impl LiveView {
    /// The Live view of `sessions`, its feeds tasks of an owner under
    /// `owner`, whose panic ends what they feed and not the engine
    /// (ADR 0148).
    pub(crate) fn new(
        sessions: Arc<[Arc<dyn WatchedSessions>]>,
        owner: &TaskOwner,
        clock: Clock,
    ) -> Self {
        Self(Arc::new(Inner {
            sessions,
            owner: owner.child_for(OwnerKind::Connection),
            clock,
            watches: Mutex::new(Watches::default()),
        }))
    }

    /// Admits a viewer of `session` on `screen`, the first one starting its
    /// capture by one of `tiers`, unless `limit` viewers watch it already.
    ///
    /// # Errors
    ///
    /// Why it was not admitted.
    pub(super) fn watch(
        &self,
        session: &SessionId,
        screen: LiveViewScreen,
        limit: u64,
        tiers: Tiers,
    ) -> Result<Viewing, NotWatched> {
        let mut watches = self.0.watches();
        let viewer = watches.next_number();
        if let Some(watch) = watches.open.get(session) {
            let mut admitted = false;
            watch.viewers.send_if_modified(|viewers| {
                admitted = u64::try_from(viewers.len()).is_ok_and(|viewers| viewers < limit);
                if admitted {
                    viewers.insert(viewer, screen);
                }
                admitted
            });
            return if admitted {
                Ok(Viewing::new(viewer, watch))
            } else {
                Err(NotWatched::ViewerLimit)
            };
        }
        let found = self
            .0
            .sessions
            .iter()
            .find_map(|sessions| sessions.watch(session))
            .ok_or(NotWatched::UnknownSession)?;
        let mut sources: VecDeque<_> = found
            .sources
            .into_iter()
            .filter(|source| tiers.take(source.tier()))
            .collect();
        let source = sources.pop_front().ok_or(NotWatched::NoTier)?;
        let mut tabs = found.tabs;
        let open: Arc<[PageId]> = tabs.borrow_and_update().as_slice().into();
        let (showing, shown) = watch::channel(Showing {
            tier: source.tier(),
            shown: open.last().cloned(),
            tabs: open,
            frame: None,
            ended: None,
        });
        let (viewers, watching) = watch::channel(Viewers::from([(viewer, screen)]));
        let watch = Watch {
            feed: watches.next_number(),
            viewers: Arc::new(viewers),
            showing: shown,
        };
        let viewing = Viewing::new(viewer, &watch);
        let feed = Feed {
            kept: Kept {
                inner: Arc::clone(&self.0),
                session: session.clone(),
                feed: watch.feed,
            },
            tabs,
            gone: BTreeSet::new(),
            source,
            sources,
            capturing: None,
            frames: 0,
            viewers: watching,
            showing,
        };
        watches.open.insert(session.clone(), watch);
        drop(watches);
        self.0.owner.spawn(feed.run());
        Ok(viewing)
    }

    /// Ends every feed and waits until each stopped its capture.
    pub(crate) async fn stop(&self) {
        self.0.owner.stop().await;
    }
}

impl Inner {
    fn watches(&self) -> MutexGuard<'_, Watches> {
        self.watches.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A feed's entry among the watched Sessions, removed with it: a viewer
/// that comes after starts another feed.
struct Kept {
    inner: Arc<Inner>,
    session: SessionId,
    feed: u64,
}

impl Kept {
    /// Removes the entry while nobody watches; `false` when a viewer joined
    /// in the meantime, which the feed then goes on for.
    fn retire(&self) -> bool {
        let mut watches = self.inner.watches();
        let still_viewed = watches
            .open
            .get(&self.session)
            .is_some_and(|watch| !watch.viewers.borrow().is_empty());
        if !still_viewed {
            watches.open.remove(&self.session);
        }
        !still_viewed
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        let mut watches = self.inner.watches();
        if watches
            .open
            .get(&self.session)
            .is_some_and(|watch| watch.feed == self.feed)
        {
            watches.open.remove(&self.session);
        }
    }
}

/// What feeds one watched Session.
struct Feed {
    kept: Kept,
    tabs: watch::Receiver<Vec<PageId>>,
    /// The tabs whose page its browser said is gone, until the Session
    /// drops them.
    gone: BTreeSet<PageId>,
    source: Box<dyn FrameSource>,
    /// The tiers left, the best first.
    sources: VecDeque<Box<dyn FrameSource>>,
    /// What the source captures, once it started.
    capturing: Option<(PageId, FrameSize)>,
    /// How many frames it published, which numbers the next.
    frames: u64,
    viewers: watch::Receiver<Viewers>,
    showing: watch::Sender<Showing>,
}

impl Feed {
    async fn run(mut self) {
        let ended = self.feed().await;
        self.stop_capture().await;
        let Self { kept, showing, .. } = self;
        // The entry goes first, so a viewer told of the end and back again
        // starts another feed.
        drop(kept);
        if let Some(ended) = ended {
            showing.send_modify(|showing| showing.ended = Some(ended));
        }
    }

    /// Feeds the viewers until the last left, or until the end it answers.
    async fn feed(&mut self) -> Option<LiveViewEnd> {
        let stopping = self.kept.inner.owner.token().clone();
        loop {
            let size = FrameSize::filling(self.viewers.borrow_and_update().values());
            let Some(size) = size else {
                self.stop_capture().await;
                if self.kept.retire() {
                    return None;
                }
                continue;
            };
            let shown = self.read_tabs();
            let wanted = shown.map(|page| (page, size));
            if wanted != self.capturing {
                self.stop_capture().await;
                if let Some((page, size)) = wanted {
                    let started = tokio::select! {
                        biased;
                        () = stopping.cancelled() => return Some(LiveViewEnd::DaemonStopping),
                        started = self.start(&page, size) => started,
                    };
                    if let Err(failure) = started {
                        if let Some(ended) = self.failed(page, failure).await {
                            return Some(ended);
                        }
                        continue;
                    }
                    self.capturing = Some((page, size));
                }
            }
            let next = tokio::select! {
                biased;
                () = stopping.cancelled() => return Some(LiveViewEnd::DaemonStopping),
                changed = self.tabs.changed() => match changed {
                    Ok(()) => continue,
                    Err(_) => return Some(LiveViewEnd::SessionClosed),
                },
                // Its sender is the entry's, which this feed removes.
                _ = self.viewers.changed() => continue,
                next = self.source.next(), if self.capturing.is_some() => next,
            };
            match next {
                Ok(frame) => self.publish(frame),
                Err(failure) => {
                    if let Some((page, _)) = self.capturing.clone()
                        && let Some(ended) = self.failed(page, failure).await
                    {
                        return Some(ended);
                    }
                }
            }
        }
    }

    /// The tab to show, the newest whose page is not gone, every viewer
    /// told the tabs and it when either changed. A frame of another page
    /// is never sent after it.
    fn read_tabs(&mut self) -> Option<PageId> {
        let tabs: Vec<PageId> = self.tabs.borrow_and_update().clone();
        self.gone.retain(|page| tabs.contains(page));
        let shown = tabs
            .iter()
            .rev()
            .find(|page| !self.gone.contains(*page))
            .cloned();
        self.showing.send_if_modified(|showing| {
            let changed = *showing.tabs != *tabs || showing.shown != shown;
            if changed {
                showing.tabs = tabs.into();
                showing.shown.clone_from(&shown);
                showing
                    .frame
                    .take_if(|frame| showing.shown.as_ref() != Some(&frame.page));
            }
            changed
        });
        shown
    }

    /// Starts capturing `page` at `size`, within [`START_WITHIN`].
    async fn start(&mut self, page: &PageId, size: FrameSize) -> Result<(), WatchFailure> {
        let clock = self.kept.inner.clock;
        tokio::select! {
            biased;
            started = self.source.start(page, size) => started,
            () = clock.sleep(START_WITHIN) => Err(WatchFailure::CaptureTimedOut),
        }
    }

    /// Stops the capture, if one started, waiting at most [`START_WITHIN`].
    async fn stop_capture(&mut self) {
        if self.capturing.take().is_some() {
            self.stop_source().await;
        }
    }

    async fn stop_source(&mut self) {
        let clock = self.kept.inner.clock;
        tokio::select! {
            biased;
            () = self.source.stop() => {}
            () = clock.sleep(START_WITHIN) => {}
        }
    }

    /// What a failed capture of `page` comes to: another tab, another tier,
    /// or the end it answers.
    async fn failed(&mut self, page: PageId, failure: WatchFailure) -> Option<LiveViewEnd> {
        self.capturing = None;
        self.stop_source().await;
        if self.tabs.has_changed().is_err() {
            return Some(LiveViewEnd::SessionClosed);
        }
        match failure {
            WatchFailure::PageGone => {
                self.gone.insert(page);
                None
            }
            WatchFailure::CaptureRefused(_)
            | WatchFailure::CaptureTimedOut
            | WatchFailure::Unrecognized(_) => self.fall_back(&failure),
            // No source knows a viewer; a tier that says one is gone lost
            // its capture as much.
            WatchFailure::BrowserGone | WatchFailure::ViewerGone => {
                tracing::warn!(%failure, "a Live view lost its Session's capture");
                Some(LiveViewEnd::CaptureLost)
            }
        }
    }

    /// Takes the next tier after `failure`, telling every viewer; the end
    /// when none is left.
    fn fall_back(&mut self, failure: &WatchFailure) -> Option<LiveViewEnd> {
        let failed = self.source.tier();
        let Some(source) = self.sources.pop_front() else {
            tracing::warn!(%failure, tier = ?failed, "a Live view lost its Session's capture");
            return Some(LiveViewEnd::CaptureLost);
        };
        self.source = source;
        let tier = self.source.tier();
        tracing::warn!(%failure, tier = ?failed, next = ?tier, "a Live view falls back to another tier");
        self.showing.send_modify(|showing| showing.tier = tier);
        None
    }

    /// Publishes `frame` as the newest.
    fn publish(&mut self, frame: SourceFrame) {
        let Some((page, _)) = &self.capturing else {
            return;
        };
        self.frames += 1;
        let frame = Arc::new(Frame::new(self.frames, page.clone(), frame));
        self.showing
            .send_modify(|showing| showing.frame = Some(frame));
    }
}

/// One viewer's side of a watched Session: what it was told and how far it
/// acknowledged. Dropping it leaves.
pub(super) struct Viewing {
    viewer: u64,
    viewers: Arc<watch::Sender<Viewers>>,
    showing: watch::Receiver<Showing>,
    tier: LiveViewTier,
    /// The tabs and the page shown it was told; none before the first.
    tabs: Option<(Arc<[PageId]>, Option<PageId>)>,
    /// The last frame sent to it, 0 before the first.
    sent: u64,
    /// Whether it has that frame to acknowledge still.
    unacknowledged: bool,
}

impl Viewing {
    fn new(viewer: u64, watch: &Watch) -> Self {
        let mut showing = watch.showing.clone();
        showing.mark_changed();
        let tier = showing.borrow().tier;
        Self {
            viewer,
            viewers: Arc::clone(&watch.viewers),
            showing,
            tier,
            tabs: None,
            sent: 0,
            unacknowledged: false,
        }
    }

    /// The tier it was last told.
    pub(super) fn tier(&self) -> LiveViewTier {
        self.tier
    }

    /// The viewer's screen is `screen` now.
    pub(super) fn set_screen(&self, screen: LiveViewScreen) {
        self.viewers.send_if_modified(|viewers| {
            let before = viewers.insert(self.viewer, screen);
            before != Some(screen)
        });
    }

    /// The viewer took frame `frame`: an earlier one's number, or one it
    /// was never sent, changes nothing.
    pub(super) fn acknowledge(&mut self, frame: u64) {
        if frame == self.sent {
            self.unacknowledged = false;
        }
    }

    /// Waits until the Session shows something it was not told.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no change.
    pub(super) async fn changed(&mut self) {
        // With no feed left, what it ended with is what `news` tells next.
        let _ = self.showing.changed().await;
    }

    /// What the viewer was not told yet but its frames, an `ended` last.
    pub(super) fn news(&mut self) -> Vec<LiveViewEngineMessage> {
        let mut news = Vec::new();
        let lost = self.showing.has_changed().is_err();
        let showing = self.showing.borrow_and_update();
        if showing.tier != self.tier {
            self.tier = showing.tier;
            news.push(LiveViewEngineMessage::Tier { tier: self.tier });
        }
        let told = self
            .tabs
            .as_ref()
            .is_some_and(|(tabs, shown)| *tabs == showing.tabs && *shown == showing.shown);
        if !told {
            let (tabs, shown) = (Arc::clone(&showing.tabs), showing.shown.clone());
            news.push(LiveViewEngineMessage::Tabs {
                tabs: tabs
                    .iter()
                    .map(|page| LiveViewTab { page: page.clone() })
                    .collect(),
                shown: shown.clone(),
            });
            self.tabs = Some((tabs, shown));
        }
        // A feed gone without a word panicked.
        let ended = showing
            .ended
            .or_else(|| lost.then_some(LiveViewEnd::CaptureLost));
        if let Some(reason) = ended {
            news.push(LiveViewEngineMessage::Ended { reason });
        }
        news
    }

    /// Sends the newest frame on `output` when the viewer has none to
    /// acknowledge and has not got it.
    ///
    /// # Errors
    ///
    /// Why the output failed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may have sent the frame, which then counts as
    /// sent.
    pub(super) async fn send_due(
        &mut self,
        output: &mut impl VideoOutput,
    ) -> Result<(), WatchFailure> {
        if self.unacknowledged {
            return Ok(());
        }
        let newest = self.showing.borrow().frame.clone();
        let Some(frame) = newest.filter(|frame| frame.number > self.sent) else {
            return Ok(());
        };
        self.sent = frame.number;
        self.unacknowledged = true;
        output.send(&frame).await
    }
}

impl Drop for Viewing {
    fn drop(&mut self) {
        self.viewers.send_if_modified(|viewers| {
            viewers.remove(&self.viewer);
            true
        });
    }
}

/// The Session `watched` would be, for the tests that name none.
#[cfg(test)]
pub(super) fn watched(
    tabs: watch::Receiver<Vec<PageId>>,
    sources: Vec<Box<dyn FrameSource>>,
) -> super::WatchedSession {
    super::WatchedSession { tabs, sources }
}
