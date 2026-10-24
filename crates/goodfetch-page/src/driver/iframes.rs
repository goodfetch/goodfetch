//! A page's out-of-process iframes on Chromium: the browser attaches the
//! engine to each one before it runs (`Target.setAutoAttach` with
//! `waitForDebuggerOnStart`), on the session of the target that holds its
//! owner element, the page's or another iframe's. Each part of the engine
//! that follows a page's frames registers what it does to one
//! ([`Iframes::on_attach`]): the interception holds its requests (ADR 0047),
//! the Consent prompt runner starts in it (ADR 0048). Every part prepares
//! the frame, in the order they registered, while the browser holds it, and
//! only then does it run, its own iframes attached under it the same way.
//! A frame a part could not prepare is never let run, so no request leaves
//! it outside the interception (ADR 0014: the domain policy fails closed).
#![expect(
    clippy::disallowed_types,
    reason = "each lock guards one list or map for one operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_cdp::generated::{runtime, target};
use goodfetch_cdp::{SessionId as TargetSession, Subscription};
use goodfetch_engine::CallError;
use goodfetch_engine::runs::Run;
use tokio_util::sync::CancellationToken;

use super::PageDriver;

/// The target type of an out-of-process iframe.
const IFRAME: &str = "iframe";

/// One out-of-process iframe of a page, as the browser holds it before it
/// runs.
#[derive(Clone)]
pub(crate) struct Iframe {
    /// The page's driver, its commands and events on the iframe's own
    /// session: its main frame is still the page's, so none of the
    /// iframe's requests counts as the page's navigation.
    pub(crate) driver: PageDriver,
    /// The session of the target that holds its owner element.
    pub(crate) parent: TargetSession,
    /// Its target's ID, which is also its frame's.
    pub(crate) target: target::TargetID,
    /// The URL the browser named when it attached it.
    pub(crate) url: String,
    /// Cancelled once its target goes, or its Run ends.
    pub(crate) gone: CancellationToken,
}

/// What one part of the engine does to each out-of-process iframe of a
/// page before it runs; whether it is ready to run. One that is not stays
/// held.
pub(crate) type OnIframe =
    Arc<dyn Fn(Iframe) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// One page's out-of-process iframes: what each part does to one, and the
/// token of each one attached, by its session.
#[derive(Clone, Default)]
pub(crate) struct Iframes {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    preparers: Mutex<Vec<OnIframe>>,
    attached: Mutex<BTreeMap<TargetSession, CancellationToken>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Iframes {
    /// Has `preparer` prepare each out-of-process iframe attached from
    /// [`Self::follow`] on, after the parts that registered before it.
    pub(crate) fn on_attach(&self, preparer: OnIframe) {
        lock(&self.shared.preparers).push(preparer);
    }

    /// Starts following the out-of-process iframes of the page `driver`
    /// drives for `run`, once every part registered what it does to one.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses to attach them.
    pub(crate) async fn follow(&self, driver: &PageDriver, run: &Run) -> Result<(), CallError> {
        let session = driver.session().clone();
        let events = driver.events_on(&session).await;
        listen(self.clone(), driver.clone(), session, events, run);
        driver.opening_command(&auto_attach()).await.map(drop)
    }
}

/// The command that has the browser attach the engine to a target's
/// out-of-process iframes, each held until the engine lets it run.
fn auto_attach() -> target::SetAutoAttachParams {
    target::SetAutoAttachParams {
        auto_attach: true,
        wait_for_debugger_on_start: true,
        flatten: Some(true),
        filter: Some(vec![
            target::FilterEntry {
                exclude: Some(false),
                r#type: Some(IFRAME.to_owned()),
            },
            target::FilterEntry {
                exclude: Some(true),
                r#type: None,
            },
        ]),
    }
}

/// Reads `session`'s events, the page's or an iframe's, as a task of the
/// Run's, boxed, since an iframe attached under it is read the same way.
fn listen(
    iframes: Iframes,
    page: PageDriver,
    session: TargetSession,
    events: Subscription,
    run: &Run,
) {
    let listening: Pin<Box<dyn Future<Output = ()> + Send>> =
        Box::pin(read(iframes, page, session, events, run.clone()));
    run.spawn(listening);
}

/// Hands each iframe the browser attached under `session` to every part,
/// and lets each one that goes go.
async fn read(
    iframes: Iframes,
    page: PageDriver,
    session: TargetSession,
    mut events: Subscription,
    run: Run,
) {
    while let Some(event) = events.next().await {
        if let Some(attached) = event.read::<target::AttachedToTargetEvent>() {
            if attached.target_info.r#type == IFRAME {
                run.spawn(attach(
                    iframes.clone(),
                    page.clone(),
                    session.clone(),
                    attached,
                    run.clone(),
                ));
            }
        } else if let Some(detached) = event.read::<target::DetachedFromTargetEvent>()
            && let Some(gone) =
                lock(&iframes.shared.attached).remove(&TargetSession(detached.session_id))
        {
            gone.cancel();
        }
    }
}

/// Prepares the iframe `attached`, held under `parent`, by every part in
/// turn, follows its own iframes, and lets it run; one a part could not
/// prepare stays held.
async fn attach(
    iframes: Iframes,
    page: PageDriver,
    parent: TargetSession,
    attached: target::AttachedToTargetEvent,
    run: Run,
) {
    let session = TargetSession(attached.session_id);
    let gone = run.cancellation().child_token();
    lock(&iframes.shared.attached).insert(session.clone(), gone.clone());
    let iframe = Iframe {
        driver: page.on_iframe(session.clone()),
        parent,
        target: attached.target_info.target_id,
        url: attached.target_info.url,
        gone,
    };
    let preparers = lock(&iframes.shared.preparers).clone();
    for preparer in preparers {
        if !preparer(iframe.clone()).await {
            tracing::warn!(url = %iframe.url, "an out-of-process iframe could not be prepared, so it does not run");
            return;
        }
    }
    let events = page.events_on(&session).await;
    listen(iframes, page.clone(), session.clone(), events, &run);
    if let Err(failure) = page.command_on(&session, &auto_attach()).await {
        tracing::debug!(%failure, "an out-of-process iframe went before its own iframes were followed");
    }
    if attached.waiting_for_debugger
        && let Err(failure) = page
            .command_on(&session, &runtime::RunIfWaitingForDebuggerParams {})
            .await
    {
        tracing::debug!(%failure, "an out-of-process iframe went before it was let run");
    }
}
