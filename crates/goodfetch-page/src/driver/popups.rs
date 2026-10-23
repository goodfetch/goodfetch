//! The popups a browser's pages open (ADR 0085; pitfall B15): the browser
//! attaches the engine to every new page target before it runs
//! (`Target.setAutoAttach` with `waitForDebuggerOnStart`), so a popup, a
//! target its opener names, is prepared as a page and handed to its
//! Session before its first request leaves; only then is it let run. The
//! Session puts it under its interception and announces it, so a listener
//! armed before the click that opened it never misses it, and the
//! Session's routes catch its first request. A page target the engine
//! opened itself is let run at once and its extra session let go, the
//! engine driving it through its own; so is a popup of a browser context
//! no Session follows.
#![expect(
    clippy::disallowed_types,
    reason = "each context's handler is set once and read by the browser's one event reader, each lock held for one map operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::{browser, runtime, target};
use tokio::sync::watch;

use super::{Browser, PageDriver, prepare};
use crate::page_value::PageValue;

/// The target type of a page.
pub(super) const PAGE: &str = "page";

/// What a browser context's Session does with a popup one of its pages
/// opened, before it runs: given the popup's driver and its opener's
/// target, it takes the popup in as a page of the Session.
pub(crate) type OnPopup = Arc<
    dyn Fn(PageDriver, target::TargetID) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

/// Each browser context's handler of its popups.
#[derive(Default)]
pub(super) struct Popups {
    contexts: Mutex<BTreeMap<browser::BrowserContextID, OnPopup>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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
    /// as every page is; its Session's proxy and navigations' deadline are
    /// the ones [`Self::for_session`] gives it.
    async fn popup(
        browser: Browser,
        session: TargetSession,
        context: browser::BrowserContextID,
        popup: target::TargetID,
    ) -> Result<Self, goodfetch_engine::CallError> {
        prepare(&browser, &session).await?;
        Ok(Self {
            browser,
            session,
            proxy: None,
            context,
            frame: popup,
            navigation_timeout: Duration::ZERO,
            started: Arc::new(watch::Sender::new(0)),
            navigating: Arc::new(watch::Sender::new(0)),
            turn: Arc::new(tokio::sync::Mutex::new(None)),
            gesture: Arc::new(tokio::sync::Mutex::new(())),
            value: PageValue::empty(),
            waits: None,
        })
    }

    /// This driver with its Session's `proxy`, whose challenges it answers,
    /// and its navigations' deadline, `navigation_timeout`.
    pub(crate) fn for_session(
        self,
        proxy: Option<Arc<crate::proxy::SessionProxy>>,
        navigation_timeout: Duration,
    ) -> Self {
        Self {
            proxy,
            navigation_timeout,
            ..self
        }
    }
}

/// The page target `attached` waits for the engine: a popup of a context a
/// Session follows is handed to it first; then it runs, and a page the
/// engine opened itself keeps only the engine's own session.
pub(super) async fn attach(browser: Browser, attached: target::AttachedToTargetEvent) {
    let session = TargetSession(attached.session_id);
    let info = attached.target_info;
    let handler = info
        .browser_context_id
        .as_ref()
        .and_then(|context| lock(&browser.popups.contexts).get(context).cloned());
    let popup = match (info.opener_id, handler, info.browser_context_id) {
        (Some(opener), Some(handler), Some(context)) => Some((opener, handler, context)),
        _ => None,
    };
    let Some((opener, handler, context)) = popup else {
        resume(&browser, &session, attached.waiting_for_debugger).await;
        return;
    };
    match PageDriver::popup(browser.clone(), session.clone(), context, info.target_id).await {
        Ok(driver) => handler(driver, opener).await,
        Err(error) => tracing::debug!(%error, "a popup went before it was prepared"),
    }
    resume(&browser, &session, attached.waiting_for_debugger).await;
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
