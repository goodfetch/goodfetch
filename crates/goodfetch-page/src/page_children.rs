//! A page's out-of-process frames and dedicated workers (ADRs 0014 and
//! 0085): the browser attaches the engine to each one before it runs, on
//! the page's target and on each of its frames' in turn, and each is
//! handed to every handler the page gave, in order, then let run. A
//! frame's own requests pass no interception of its page's target, so the
//! interception holds each before its first request leaves; a dedicated
//! worker's pass its page's, but it waits, as every target the browser
//! attaches does, until the engine lets it run. A child a handler could
//! not hold is never let run: the domain policy fails closed (ADR 0014).
//!
//! One task of the Page's owner follows them all (ADR 0146), so the
//! Page's close ends it, and marks each child gone with it. A page's
//! popups are its browser's to follow, not its own.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::stream::{self, FuturesUnordered, SelectAll, Stream, StreamExt as _};
use goodfetch_engine::CallError;
use goodfetch_engine::tasks::TaskStarter;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

pub(crate) use crate::cdp::ChildKind;
use crate::cdp::{AttachedChild, CdpPage, ChildEvent, ChildEvents, TargetSession};

/// A child of a page the browser attached the engine to, waiting to run.
pub(crate) struct Child {
    pub(crate) kind: ChildKind,
    /// The session of the target it is a child of: the page's, or a frame's.
    pub(crate) parent: TargetSession,
    /// Its own session.
    pub(crate) session: TargetSession,
    /// Its target.
    pub(crate) target: String,
    /// The URL the browser named when it attached it.
    pub(crate) url: String,
    /// Cancelled once the browser detached it, or its page closed.
    pub(crate) gone: CancellationToken,
}

/// What one of a page's parts does with each child before it runs: it
/// answers whether it holds the child as it must, so it may run.
pub(crate) type OnChild =
    Arc<dyn Fn(&Child) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// What one of a page's targets tells of its children, each told with the
/// session of the target that told it.
type Told = Pin<Box<dyn Stream<Item = (TargetSession, ChildEvent)> + Send>>;

/// Follows the out-of-process frames and dedicated workers of the page
/// `cdp` drives from now on, as a task of `tasks`, its owner's, in `span`:
/// each, attached before it runs, is handed to `handlers` in order, then
/// let run once every one of them held it.
///
/// # Errors
///
/// The code of opening a page when the browser refuses to attach.
pub(crate) async fn follow(
    cdp: &CdpPage,
    (tasks, span): (&TaskStarter, &tracing::Span),
    handlers: Vec<OnChild>,
) -> Result<(), CallError> {
    let page = cdp.session().clone();
    let told = told_by(page.clone(), cdp.child_events(&page));
    tasks.spawn(
        following(cdp.clone(), told, handlers.into(), tasks.child_token()).instrument(span.clone()),
    );
    cdp.attach_children_opening().await
}

/// What `parent`'s `events` tell, each with `parent`.
fn told_by(parent: TargetSession, events: ChildEvents) -> Told {
    Box::pin(stream::unfold(
        (parent, events),
        |(parent, mut events)| async move {
            let event = events.next().await?;
            Some(((parent.clone(), event), (parent, events)))
        },
    ))
}

/// Reads what the page's target, `page`, and each of its frames' tell of
/// their children, until `token` is cancelled, or every one of them went
/// and every child is let run: each child attached is prepared beside the
/// others, and marked gone once the browser detaches it.
async fn following(cdp: CdpPage, page: Told, handlers: Arc<[OnChild]>, token: CancellationToken) {
    let mut told = SelectAll::new();
    told.push(page);
    // A frame's own children, which its preparation starts following.
    let (frames, mut new_frames) = mpsc::unbounded_channel();
    let mut attaching = FuturesUnordered::new();
    let mut attached: HashMap<TargetSession, CancellationToken> = HashMap::new();
    loop {
        // A preparation hands its frame's events in before it ends.
        while let Ok(frame) = new_frames.try_recv() {
            told.push(frame);
        }
        if told.is_empty() && attaching.is_empty() {
            return;
        }
        tokio::select! {
            biased;
            () = token.cancelled() => return,
            Some(frame) = new_frames.recv() => told.push(frame),
            Some(()) = attaching.next(), if !attaching.is_empty() => {}
            event = told.next(), if !told.is_empty() => match event {
                // Each target went.
                None => {}
                Some((_, ChildEvent::Detached(session))) => {
                    if let Some(gone) = attached.remove(&session) {
                        gone.cancel();
                    }
                }
                Some((parent, ChildEvent::Attached(child))) => {
                    let gone = token.child_token();
                    attached.insert(child.session.clone(), gone.clone());
                    attaching.push(attach(
                        &cdp,
                        (parent, child, gone),
                        Arc::clone(&handlers),
                        frames.clone(),
                    ));
                }
            },
        }
    }
}

/// Prepares the child `attached` of `parent`, marked gone by `gone`, and
/// lets it run: a frame's own children are followed first, its events
/// handed to `frames`, then every handler takes it in turn.
async fn attach(
    cdp: &CdpPage,
    (parent, attached, gone): (TargetSession, AttachedChild, CancellationToken),
    handlers: Arc<[OnChild]>,
    frames: mpsc::UnboundedSender<Told>,
) {
    let AttachedChild {
        kind,
        session,
        target,
        url,
        waiting,
    } = attached;
    if kind == ChildKind::Frame {
        // Its loop outlives every preparation.
        let _ = frames.send(told_by(session.clone(), cdp.child_events(&session)));
        if let Err(failure) = cdp.attach_children(&session).await {
            tracing::debug!(%failure, "an out-of-process frame went before its own children were followed");
        }
    }
    let child = Child {
        kind,
        parent,
        session,
        target,
        url,
        gone,
    };
    let mut held = true;
    for handler in handlers.iter() {
        held &= handler(&child).await;
    }
    if !held {
        tracing::warn!(
            kind = ?child.kind,
            "a page's child the interception could not hold does not run"
        );
        return;
    }
    if waiting && let Err(failure) = cdp.let_run(&child.session).await {
        tracing::debug!(%failure, "a page's child went before it was let run");
    }
}
