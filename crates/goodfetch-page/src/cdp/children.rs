//! The primitives that follow a page's out-of-process frames and dedicated
//! workers (ADRs 0014 and 0085): the browser attaches the engine to each
//! one before it runs (`Target.setAutoAttach` with
//! `waitForDebuggerOnStart`, on the page's target and on each of its
//! frames' in turn), tells when it goes, and lets it run once the engine
//! says so.

use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::{runtime, target};
use goodfetch_engine::CallError;

use super::paused::NotTaken;
use super::{CdpPage, TargetSession};

/// The target type of a frame the browser runs out of process.
const FRAME: &str = "iframe";

/// The target type of a dedicated worker.
const WORKER: &str = "worker";

/// What kind of child of a page a target is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildKind {
    /// A frame the browser runs out of process.
    Frame,
    /// A dedicated worker of the page or of one of its frames.
    Worker,
}

/// A child of a page the browser attached the engine to, as it told it.
pub(crate) struct AttachedChild {
    pub(crate) kind: ChildKind,
    /// Its own session.
    pub(crate) session: TargetSession,
    /// Its target.
    pub(crate) target: String,
    /// The URL the browser named when it attached it.
    pub(crate) url: String,
    /// Whether it waits for the engine to let it run.
    pub(crate) waiting: bool,
}

/// What a page's target, or a frame's, tells of its children.
pub(crate) enum ChildEvent {
    /// A child was attached.
    Attached(AttachedChild),
    /// The child on this session was detached.
    Detached(TargetSession),
}

impl ChildEvent {
    fn of(event: &goodfetch_cdp::RawEvent) -> Option<Self> {
        if let Some(detached) = event.read::<target::DetachedFromTargetEvent>() {
            return Some(Self::Detached(TargetSession(detached.session_id)));
        }
        let attached = event.read::<target::AttachedToTargetEvent>()?;
        let kind = match attached.target_info.r#type.as_str() {
            FRAME => ChildKind::Frame,
            WORKER => ChildKind::Worker,
            _ => return None,
        };
        Some(Self::Attached(AttachedChild {
            kind,
            session: TargetSession(attached.session_id),
            target: attached.target_info.target_id,
            url: attached.target_info.url,
            waiting: attached.waiting_for_debugger,
        }))
    }
}

/// What one of a page's targets tells of its children.
pub(crate) struct ChildEvents(Subscription);

impl ChildEvents {
    /// The next thing the target tells of its children; `None` once it
    /// went.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub(crate) async fn next(&mut self) -> Option<ChildEvent> {
        while let Some(event) = self.0.next().await {
            if let Some(told) = ChildEvent::of(&event) {
                return Some(told);
            }
        }
        None
    }
}

impl CdpPage {
    /// The session of the page's own target, its first children's parent.
    pub(crate) fn session(&self) -> &TargetSession {
        self.page.session()
    }

    /// What `parent`, the page's target or one of its frames', tells of its
    /// children from now.
    pub(crate) fn child_events(&self, parent: &TargetSession) -> ChildEvents {
        ChildEvents(self.page.events_on(parent))
    }

    /// Has the browser attach the engine to each of the page's
    /// out-of-process frames and dedicated workers before it runs, as one
    /// of the commands that open the page.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn attach_children_opening(&self) -> Result<(), CallError> {
        self.page.opening_command(&auto_attach()).await.map(drop)
    }

    /// Has the browser attach the engine to each of the out-of-process
    /// frames and dedicated workers of the frame on `frame` before it runs.
    ///
    /// # Errors
    ///
    /// When the frame went.
    pub(crate) async fn attach_children(&self, frame: &TargetSession) -> Result<(), NotTaken> {
        Ok(self
            .page
            .command_on(frame, &auto_attach())
            .await
            .map(drop)?)
    }

    /// Has the child this page drives, of kind `kind`, show its headless
    /// browser's identity, as its page does, within a deadline of opening a
    /// page; nothing with a window.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn show_identity_opening(&self, kind: ChildKind) -> Result<(), CallError> {
        self.page.show_identity_opening(kind).await
    }

    /// Lets the child on `child`, which waits for the engine, run.
    ///
    /// # Errors
    ///
    /// When the child went.
    pub(crate) async fn let_run(&self, child: &TargetSession) -> Result<(), NotTaken> {
        Ok(self
            .page
            .command_on(child, &runtime::RunIfWaitingForDebuggerParams {})
            .await
            .map(drop)?)
    }
}

/// The auto-attach of a page's or a frame's target: its out-of-process
/// frames and dedicated workers, each waiting for the engine.
fn auto_attach() -> target::SetAutoAttachParams {
    let include = |kind: &str| target::FilterEntry {
        exclude: Some(false),
        r#type: Some(kind.to_owned()),
    };
    target::SetAutoAttachParams {
        auto_attach: true,
        wait_for_debugger_on_start: true,
        flatten: Some(true),
        filter: Some(vec![
            include(FRAME),
            include(WORKER),
            target::FilterEntry {
                exclude: Some(true),
                r#type: None,
            },
        ]),
    }
}
