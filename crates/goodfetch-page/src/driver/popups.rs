//! The popups a browser's pages open (ADR 0085; pitfall B15): the browser
//! attaches the engine to every new page target before it runs
//! (`Target.setAutoAttach` with `waitForDebuggerOnStart`), so a popup, a
//! target its opener names, is prepared as a page and handed to its
//! Session before its first request leaves; only then is it let run. The
//! Session puts it under its interception and announces it, so a listener
//! armed before the click that opened it never misses it, and the
//! Session's routes catch its first request; one the Session cannot take
//! in, its opener or its Session gone meanwhile, is closed before it runs.
//! A page target the engine opened itself is let run at once and its extra
//! session let go, the engine driving it through its own; so is a popup of
//! a browser context no Session follows, and, on a Borrowed browser, whose
//! context the person's own windows share, any page target no page of the
//! Session opened (ADR 0056).
#![expect(
    clippy::disallowed_types,
    reason = "each context's handler is set once and read by the browser's one event reader, each lock held for one map operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::{browser, runtime, target};
use tokio::sync::watch;

use super::{Browser, PageDriver, prepare};
use crate::errors::Call;
use crate::page_value::PageValue;

/// The target type of a page.
pub(super) const PAGE: &str = "page";

/// What a browser context's Session does with a page target one of the
/// context's pages opened, before it runs.
pub(crate) trait PopupHandler: Send + Sync {
    /// Whether the page whose target is `opener` is, or was, one of the
    /// Session's: only such a page's popups are the Session's.
    fn holds(&self, opener: target::TargetID) -> Pin<Box<dyn Future<Output = bool> + Send>>;

    /// Takes the popup `driver` drives, which the page whose target is
    /// `opener` opened, in as a page of the Session: whether it did.
    fn take_in(
        &self,
        driver: PageDriver,
        opener: target::TargetID,
    ) -> Pin<Box<dyn Future<Output = bool> + Send>>;
}

/// A browser context's handler of its popups.
pub(crate) type OnPopup = Arc<dyn PopupHandler>;

/// Each browser context's handler of its popups, and the page targets the
/// engine attaches to itself.
#[derive(Default)]
pub(super) struct Popups {
    contexts: Mutex<BTreeMap<browser::BrowserContextID, OnPopup>>,
    own: Mutex<Own>,
}

/// The page targets the engine is attaching to itself, and the sessions it
/// attached so, which the browser also reports as attached: the engine
/// drives such a page through that session, so none is let go of.
#[derive(Default)]
struct Own {
    /// By target, how many attaches are on their way.
    attaching: BTreeMap<target::TargetID, usize>,
    sessions: BTreeSet<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Popups {
    /// Whether the engine attached `session` to `target` itself, or is
    /// attaching to `target`.
    fn owns(&self, target: &target::TargetID, session: &TargetSession) -> bool {
        let own = lock(&self.own);
        own.attaching.contains_key(target) || own.sessions.contains(&session.0)
    }

    /// Forgets `session`, which the browser detached.
    pub(super) fn forget(&self, session: &TargetSession) {
        lock(&self.own).sessions.remove(&session.0);
    }
}

impl Browser {
    /// Attaches the engine to the page target `target_id` itself, a
    /// session it drives the page through, which is never let go of; a
    /// failure raises the code of `call`.
    pub(super) async fn attach_own(
        &self,
        target_id: &target::TargetID,
        call: &Call,
    ) -> Result<TargetSession, goodfetch_engine::CallError> {
        *lock(&self.popups.own)
            .attaching
            .entry(target_id.clone())
            .or_default() += 1;
        let attached = self
            .send(
                None,
                &target::AttachToTargetParams {
                    target_id: target_id.clone(),
                    flatten: Some(true),
                },
                call,
            )
            .await;
        let mut own = lock(&self.popups.own);
        if let Ok(attached) = &attached {
            own.sessions.insert(attached.session_id.clone());
        }
        if let Some(count) = own.attaching.get_mut(target_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                own.attaching.remove(target_id);
            }
        }
        attached.map(|attached| TargetSession(attached.session_id))
    }
}

impl PageDriver {
    /// Hands the popups of the page's browser context to `handler`, once for
    /// the context: a later handler for the same context changes nothing.
    pub(crate) fn follow_popups(&self, handler: OnPopup) {
        lock(&self.browser.popups.contexts)
            .entry(self.context.clone())
            .or_insert(handler);
    }

    /// The driver of a popup of the browser context `context`, which the
    /// browser attached on `session` and whose target is `popup`, prepared
    /// as every page is; its Session's proxy is the one
    /// [`Self::for_session`] gives it.
    async fn popup(
        browser: Browser,
        session: TargetSession,
        context: browser::BrowserContextID,
        popup: target::TargetID,
    ) -> Result<Self, goodfetch_engine::CallError> {
        prepare(&browser, &session).await?;
        Ok(Self {
            page_reader: super::PageReader::new(&browser.connection, &session),
            browser,
            session,
            egress: super::SessionEgress::default(),
            context,
            frame: popup,
            started: Arc::new(watch::Sender::new(0)),
            navigating: Arc::new(watch::Sender::new(0)),
            spent: Duration::ZERO,
            turn: Arc::new(tokio::sync::Mutex::new(None)),
            gesture: Arc::new(tokio::sync::Mutex::new(())),
            value: PageValue::empty(),
            waits: None,
            reader: None,
        })
    }

    /// This driver with its Session's `egress`, whose challenges it
    /// answers.
    pub(crate) fn for_session(self, egress: super::SessionEgress) -> Self {
        Self { egress, ..self }
    }
}

/// The page target `attached` waits for the engine: a popup a Session's
/// page opened is handed to the Session, and runs once taken in, or is
/// closed before it runs, so no page of the Session's ever runs outside its
/// interception; any other page target, one the engine opened itself or a
/// window of a Borrowed browser's person (ADR 0056), runs at once and is let
/// go of.
pub(super) async fn attach(browser: Browser, attached: target::AttachedToTargetEvent) {
    let session = TargetSession(attached.session_id);
    let waiting = attached.waiting_for_debugger;
    let info = attached.target_info;
    if !waiting && browser.popups.owns(&info.target_id, &session) {
        return;
    }
    let handler = info
        .browser_context_id
        .as_ref()
        .and_then(|context| lock(&browser.popups.contexts).get(context).cloned());
    let popup = match (info.opener_id, handler, info.browser_context_id) {
        (Some(opener), Some(handler), Some(context)) => Some((opener, handler, context)),
        _ => None,
    };
    let Some((opener, handler, context)) = popup else {
        let_go(&browser, &session, waiting).await;
        return;
    };
    if !handler.holds(opener.clone()).await {
        let_go(&browser, &session, waiting).await;
        return;
    }
    let target = info.target_id.clone();
    let taken =
        match PageDriver::popup(browser.clone(), session.clone(), context, info.target_id).await {
            Ok(driver) => handler.take_in(driver, opener).await,
            Err(error) => {
                tracing::debug!(%error, "a popup went before it was prepared");
                false
            }
        };
    if taken {
        resume(&browser, &session, waiting).await;
        return;
    }
    tracing::warn!("a popup of a Session's page was not taken in, so it is closed before it runs");
    if let Err(failure) = browser
        .try_send(None, &target::CloseTargetParams { target_id: target })
        .await
    {
        tracing::debug!(%failure, "a popup not taken in went before it was closed");
    }
}

/// Lets the target attached on `session` run, when it waits for the
/// engine.
async fn resume(browser: &Browser, session: &TargetSession, waiting: bool) {
    if waiting
        && let Err(failure) = browser
            .try_send(Some(session), &runtime::RunIfWaitingForDebuggerParams {})
            .await
    {
        tracing::debug!(%failure, "a page target went before it was let run");
    }
}

/// Lets the target attached on `session` run, then detaches the engine
/// from it: the engine keeps no session on a page it does not drive there.
async fn let_go(browser: &Browser, session: &TargetSession, waiting: bool) {
    resume(browser, session, waiting).await;
    if let Err(failure) = browser
        .try_send(
            None,
            &target::DetachFromTargetParams {
                session_id: Some(session.0.clone()),
                target_id: None,
            },
        )
        .await
    {
        tracing::debug!(%failure, "a page target went before it was let go of");
    }
}
