//! A page's out-of-process frames and dedicated workers (ADRs 0014 and
//! 0085): the browser attaches the engine to each one before it runs
//! (`Target.setAutoAttach` with `waitForDebuggerOnStart`, on the page's
//! target and on each of its frames' in turn), and each is handed to every
//! handler the page gave, in order, then let run. A frame's own requests
//! pass no interception of its page's target, so the interception holds
//! each before its first request leaves; a dedicated worker's pass its
//! page's, but it waits, as every target the browser attaches does, until
//! the engine lets it run. A child a handler could not hold is never let
//! run: the domain policy fails closed (ADR 0014).

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_cdp::generated::{runtime, target};
use goodfetch_cdp::{SessionId as TargetSession, Subscription};
use goodfetch_engine::CallError;
use goodfetch_engine::runs::Run;
use tokio_util::sync::CancellationToken;

use super::PageDriver;

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

/// A child of a page the browser attached the engine to, waiting to run.
pub(crate) struct Child {
    pub(crate) kind: ChildKind,
    /// The session of the target it is a child of: the page's, or a frame's.
    pub(crate) parent: TargetSession,
    /// Its own session.
    pub(crate) session: TargetSession,
    /// Its target.
    pub(crate) target: target::TargetID,
    /// The URL the browser named when it attached it.
    pub(crate) url: String,
    /// Cancelled once the browser detached it, or its Run ended.
    pub(crate) gone: CancellationToken,
}

/// What one of a page's parts does with each child before it runs: it
/// answers whether it holds the child as it must, so it may run.
pub(crate) type OnChild =
    Arc<dyn Fn(&Child) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

impl PageDriver {
    /// The driver of the page's child target on `session`: its commands go
    /// there, and it shares everything else with the page's, so its page's
    /// main frame is never one of its own frames.
    pub(crate) fn on_child(&self, session: TargetSession) -> Self {
        Self {
            session,
            ..self.clone()
        }
    }

    /// Follows the page's out-of-process frames and dedicated workers for
    /// `run` from now on: each, attached before it runs, is handed to
    /// `handlers` in order, then let run once every one of them held it.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses to attach.
    pub(crate) async fn follow_children(
        &self,
        run: &Run,
        handlers: Vec<OnChild>,
    ) -> Result<(), CallError> {
        let handlers: Arc<[OnChild]> = handlers.into();
        let events = self.events();
        run.spawn(follow(
            self.clone(),
            self.session.clone(),
            events,
            handlers,
            run.clone(),
        ));
        self.opening_command(&auto_attach()).await.map(drop)
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

/// Reads the events of `parent`, the page's or a frame's target, until it
/// goes: each child attached is handed on as a task of its own, and marked
/// gone once the browser detaches it. Boxed, as a frame's own children are
/// followed in turn.
fn follow(
    driver: PageDriver,
    parent: TargetSession,
    mut events: Subscription,
    handlers: Arc<[OnChild]>,
    run: Run,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        let mut attached_children: HashMap<String, CancellationToken> = HashMap::new();
        while let Some(event) = events.next().await {
            if let Some(detached) = event.read::<target::DetachedFromTargetEvent>() {
                if let Some(gone) = attached_children.remove(&detached.session_id) {
                    gone.cancel();
                }
                continue;
            }
            let Some(attached) = event.read::<target::AttachedToTargetEvent>() else {
                continue;
            };
            let kind = match attached.target_info.r#type.as_str() {
                FRAME => ChildKind::Frame,
                WORKER => ChildKind::Worker,
                _ => continue,
            };
            let gone = run.cancellation().child_token();
            attached_children.insert(attached.session_id.clone(), gone.clone());
            let child = Child {
                kind,
                parent: parent.clone(),
                session: TargetSession(attached.session_id),
                target: attached.target_info.target_id,
                url: attached.target_info.url,
                gone,
            };
            let waiting = attached.waiting_for_debugger;
            run.spawn(attach(
                driver.clone(),
                child,
                waiting,
                Arc::clone(&handlers),
                run.clone(),
            ));
        }
    })
}

/// Prepares `child` and lets it run: a frame's own children are followed
/// first, then every handler takes it in turn.
async fn attach(
    driver: PageDriver,
    child: Child,
    waiting: bool,
    handlers: Arc<[OnChild]>,
    run: Run,
) {
    if child.kind == ChildKind::Frame {
        let events = driver.events_on(&child.session);
        run.spawn(follow(
            driver.clone(),
            child.session.clone(),
            events,
            Arc::clone(&handlers),
            run.clone(),
        ));
        if let Err(failure) = driver.command_on(&child.session, &auto_attach()).await {
            tracing::debug!(%failure, "an out-of-process frame went before its own children were followed");
        }
    }
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
    if waiting
        && let Err(failure) = driver
            .command_on(&child.session, &runtime::RunIfWaitingForDebuggerParams {})
            .await
    {
        tracing::debug!(%failure, "a page's child went before it was let run");
    }
}
