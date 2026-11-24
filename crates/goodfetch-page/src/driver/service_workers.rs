//! A browser's service workers on Chromium (ADR 0121): the browser
//! attaches the engine to each one before it runs (`Target.setAutoAttach`
//! with `waitForDebuggerOnStart`), so no request leaves a worker before the
//! interception covers it. Each browser context says, once, what becomes of
//! its workers: under `allow` a worker of a site is handed to its Session's
//! interception, then let run; under `block` the fetch of every worker's
//! script is failed, so `register()` rejects as on a network failure, with
//! `register` the browser's own function and no worker ever running; one
//! that attaches all the same is closed before it runs, and a script fetch
//! whose worker's context the browser cannot name is failed too. The
//! pinned builds have no per-context switch that makes `register()` reject,
//! so this is spec #577's floor, and never a bypass of service workers.
//! Chromium fetches a worker's script outside every target's interception,
//! so that fetch is failed by the browser's own interception, which pauses
//! only the kind of request a worker's script is and knows it by the
//! `Service-Worker: script` header and the worker it loads.
//!
//! A site's shared worker, which belongs to no page either, is attached the
//! same way and handed to its context's Session, whose interception holds
//! its requests before it runs, whatever the context chose for its service
//! workers (ADR 0014).
//!
//! Any other worker (a browser extension's, or one of a context no Session
//! named) is let run untouched, but for a headless browser's identity,
//! which every worker takes before it runs, as every page does (`identity`).
#![expect(
    clippy::disallowed_types,
    reason = "each context's choice is set once and read by the browser's one event reader, each lock held for one map operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use goodfetch_cdp::generated::{browser, fetch, network, runtime, target};
use goodfetch_cdp::{SessionId as TargetSession, Subscription};
use goodfetch_engine::CallError;
use goodfetch_engine::runs::Run;
use tokio::sync::{OnceCell, watch};
use tokio_util::sync::CancellationToken;

use super::{Browser, PageDriver};

/// The target type of a service worker.
const SERVICE_WORKER: &str = "service_worker";

/// The target type of a shared worker.
const SHARED_WORKER: &str = "shared_worker";

/// The header the browser's fetch of a worker's script carries.
const SCRIPT_HEADER: &str = "service-worker";

/// What one service or shared worker's Session does once the browser
/// attached the engine to it, before it runs: given the worker's driver and
/// the token cancelled once the worker's target goes, it answers whether
/// the interception now holds the worker's requests. A worker it does not
/// hold never runs: the domain policy fails closed (ADR 0014).
pub(crate) type OnWorker = Arc<
    dyn Fn(PageDriver, CancellationToken) -> Pin<Box<dyn Future<Output = bool> + Send>>
        + Send
        + Sync,
>;

/// What becomes of a browser context's service workers.
#[derive(Clone)]
pub(crate) enum ServiceWorkers {
    /// They run, each first handed to its Session's interception.
    Allow(OnWorker),
    /// None runs: the fetch of each one's script fails.
    Block,
}

/// A browser's service workers: each context's choice, and the targets of
/// the workers it attached to.
#[derive(Default)]
pub(super) struct ServiceWorkerChoices {
    contexts: Mutex<BTreeMap<browser::BrowserContextID, ServiceWorkers>>,
    /// What each context's Session does with its shared workers.
    shared: Mutex<BTreeMap<browser::BrowserContextID, OnWorker>>,
    /// Each attached worker's token, by its session, cancelled once it goes.
    attached: Mutex<BTreeMap<TargetSession, CancellationToken>>,
    /// The browser's own interception of workers' scripts, once a context
    /// blocks its workers.
    scripts: OnceCell<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ServiceWorkerChoices {
    /// The choice of the context `context`, if one was made.
    fn of(&self, context: Option<&browser::BrowserContextID>) -> Option<ServiceWorkers> {
        lock(&self.contexts).get(context?).cloned()
    }

    /// What the context `context` does with its shared workers, if a
    /// Session said.
    fn shared_of(&self, context: Option<&browser::BrowserContextID>) -> Option<OnWorker> {
        lock(&self.shared).get(context?).cloned()
    }

    /// Lets go of the choices of the context `context`, which closed, with
    /// what they hold of its Session's Run.
    pub(super) fn forget(&self, context: &browser::BrowserContextID) {
        let forgotten = lock(&self.contexts).remove(context);
        let shared = lock(&self.shared).remove(context);
        drop((forgotten, shared));
    }
}

impl PageDriver {
    /// The driver of a service worker of the browser context `context`,
    /// which the browser attached on `session` and whose target is
    /// `worker`: only its Session's interception sends commands on it, and
    /// no navigation, gesture or document is its. Its Session's proxy is
    /// the one [`Self::with_proxy`] gives it.
    fn service_worker(
        browser: Browser,
        session: TargetSession,
        context: browser::BrowserContextID,
        worker: target::TargetID,
    ) -> Self {
        Self {
            page_reader: super::PageReader::new(&browser.connection, &session),
            browser,
            session,
            egress: super::SessionEgress::default(),
            context,
            frame: worker,
            started: Arc::new(watch::Sender::new(0)),
            navigating: Arc::new(watch::Sender::new(0)),
            spent: Duration::ZERO,
            guard_world: Arc::new(watch::Sender::new(None)),
            value: crate::page_value::PageValue::empty(),
            waits: None,
            reader: None,
        }
    }

    /// Its Session's proxy and egress relay, shared.
    pub(crate) fn egress_handle(&self) -> super::SessionEgress {
        self.egress.clone()
    }

    /// This driver with its Session's `egress`, whose challenges it
    /// answers: a service worker's requests reach the proxy, or the relay,
    /// of the context its pages are in.
    pub(crate) fn with_egress(self, egress: super::SessionEgress) -> Self {
        Self { egress, ..self }
    }

    /// Chooses, once for the page's browser context, what becomes of the
    /// service workers of its sites (ADR 0121); a later choice for the same
    /// context changes nothing. `block` turns on the browser's own
    /// interception of workers' scripts first.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses that
    /// interception.
    pub(crate) async fn service_workers(&self, choice: ServiceWorkers) -> Result<(), CallError> {
        let browser = &self.browser;
        if matches!(choice, ServiceWorkers::Block) {
            browser
                .service_workers
                .scripts
                .get_or_try_init(|| async {
                    browser
                        .send(
                            None,
                            &fetch::EnableParams {
                                patterns: Some(vec![fetch::RequestPattern {
                                    url_pattern: Some("*".to_owned()),
                                    // The kind the browser gives a worker's
                                    // script, so no page's document, script
                                    // or call pauses here.
                                    resource_type: Some(network::ResourceType::Other),
                                    request_stage: Some(fetch::RequestStage::Request),
                                }]),
                                handle_auth_requests: None,
                            },
                            &browser.opening(),
                        )
                        .await
                        .map(drop)
                })
                .await?;
        }
        lock(&browser.service_workers.contexts)
            .entry(self.context.clone())
            .or_insert(choice);
        Ok(())
    }

    /// Hands, once for the page's browser context, the shared workers of
    /// its sites to `handler` before they run; a later handler for the same
    /// context changes nothing.
    pub(crate) fn shared_workers(&self, handler: OnWorker) {
        lock(&self.browser.service_workers.shared)
            .entry(self.context.clone())
            .or_insert(handler);
    }
}

/// Starts following `browser`'s service workers and new pages for `run`:
/// its browser events are read from now, then the browser attaches the
/// engine to each worker and each page before it runs (`popups`).
///
/// # Errors
///
/// The code of opening a page when the browser refuses to attach.
pub(super) async fn start(browser: &Browser, run: &Run) -> Result<(), CallError> {
    let events = browser.connection.subscribe(None);
    run.spawn(follow(events, browser.clone(), run.clone()));
    browser
        .send(
            None,
            &target::SetAutoAttachParams {
                auto_attach: true,
                wait_for_debugger_on_start: true,
                flatten: Some(true),
                filter: Some(vec![
                    target::FilterEntry {
                        exclude: Some(false),
                        r#type: Some(SERVICE_WORKER.to_owned()),
                    },
                    target::FilterEntry {
                        exclude: Some(false),
                        r#type: Some(SHARED_WORKER.to_owned()),
                    },
                    // Pages, so a popup is prepared before it runs.
                    target::FilterEntry {
                        exclude: Some(false),
                        r#type: Some(super::popups::PAGE.to_owned()),
                    },
                    target::FilterEntry {
                        exclude: Some(true),
                        r#type: None,
                    },
                ]),
            },
            &browser.launching(),
        )
        .await
        .map(drop)
}

/// Reads the browser's own events until its connection ends: each worker
/// attached is handed on as its context chose, each one gone is let go,
/// and each script fetch the browser's interception paused is failed or
/// sent on.
async fn follow(mut events: Subscription, browser: Browser, run: Run) {
    while let Some(event) = events.next().await {
        if let Some(attached) = event.read::<target::AttachedToTargetEvent>() {
            if attached.target_info.r#type == SERVICE_WORKER {
                let (browser, task) = (browser.clone(), run.clone());
                task.spawn(attach(browser, attached, run.clone()));
            } else if attached.target_info.r#type == SHARED_WORKER {
                let choice = browser
                    .service_workers
                    .shared_of(attached.target_info.browser_context_id.as_ref())
                    .map(ServiceWorkers::Allow);
                let (browser, task) = (browser.clone(), run.clone());
                task.spawn(hold_then_run(browser, attached, run.clone(), choice));
            } else if attached.target_info.r#type == super::popups::PAGE {
                run.spawn(super::popups::attach(browser.clone(), attached));
            }
        } else if let Some(detached) = event.read::<target::DetachedFromTargetEvent>() {
            let detached = TargetSession(detached.session_id);
            browser.popups.forget(&detached);
            if let Some(gone) = lock(&browser.service_workers.attached).remove(&detached) {
                gone.cancel();
            }
        } else if let Some(paused) = event.read::<fetch::RequestPausedEvent>() {
            run.spawn(script_fetch(browser.clone(), paused));
        }
    }
}

/// The service worker `attached` waits for the engine: under `allow` its
/// Session's interception takes it first, under `block` it is closed; then
/// it runs.
async fn attach(browser: Browser, attached: target::AttachedToTargetEvent, run: Run) {
    let choice = browser
        .service_workers
        .of(attached.target_info.browser_context_id.as_ref());
    hold_then_run(browser, attached, run, choice).await;
}

/// The worker `attached` waits for the engine: a site's worker is handed
/// to its Session's interception under `choice`'s `allow`, and closed when
/// the interception could not hold it or `choice` blocks it; then a
/// headless browser's worker takes the identity its pages show, and runs.
/// The identity comes after the interception: a shared worker given its
/// user agent before the interception paused its requests sent them past
/// it.
async fn hold_then_run(
    browser: Browser,
    attached: target::AttachedToTargetEvent,
    run: Run,
    choice: Option<ServiceWorkers>,
) {
    let session = TargetSession(attached.session_id);
    let info = attached.target_info;
    let of_a_site = info.url.starts_with("http://") || info.url.starts_with("https://");
    let choice = choice.filter(|_| of_a_site);
    let held = match (choice, info.browser_context_id.clone()) {
        (Some(ServiceWorkers::Allow(handle)), Some(context)) => {
            let gone = run.cancellation().child_token();
            lock(&browser.service_workers.attached).insert(session.clone(), gone.clone());
            let driver = PageDriver::service_worker(
                browser.clone(),
                session.clone(),
                context,
                info.target_id.clone(),
            );
            run.spawn(driver.reading(None));
            handle(driver, gone).await
        }
        // A worker of a context that blocks them, whose script fetch went
        // through all the same, never runs.
        (Some(ServiceWorkers::Block), _) => false,
        (None | Some(ServiceWorkers::Allow(_)), _) => true,
    };
    if !held {
        let closed = browser
            .try_send(
                None,
                &target::CloseTargetParams {
                    target_id: info.target_id,
                },
            )
            .await;
        if let Err(failure) = closed {
            tracing::debug!(%failure, "a worker that may not run went first");
        }
        return;
    }
    if let Some(identity) = &browser.identity
        && let Err(failure) = browser
            .try_send(Some(&session), &identity.worker_command())
            .await
    {
        tracing::debug!(%failure, "a service worker went before it took its browser's identity");
    }
    if attached.waiting_for_debugger
        && let Err(failure) = browser
            .try_send(Some(&session), &runtime::RunIfWaitingForDebuggerParams {})
            .await
    {
        tracing::debug!(%failure, "a worker went before it was let run");
    }
}

/// A request the browser's own interception paused: the fetch of a
/// worker's script in a context that blocks its workers fails, as a network
/// failure does; any other goes on.
async fn script_fetch(browser: Browser, paused: fetch::RequestPausedEvent) {
    let script = paused.request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case(SCRIPT_HEADER) && value.as_str() == Some("script")
    });
    let blocked = script && blocks(&browser, &paused.frame_id).await;
    let answered = if blocked {
        browser
            .try_send(
                None,
                &fetch::FailRequestParams {
                    request_id: paused.request_id,
                    error_reason: network::ErrorReason::Failed,
                },
            )
            .await
            .map(drop)
    } else {
        browser
            .try_send(
                None,
                &fetch::ContinueRequestParams {
                    request_id: paused.request_id,
                    url: None,
                    method: None,
                    post_data: None,
                    headers: None,
                    intercept_response: None,
                },
            )
            .await
            .map(drop)
    };
    if let Err(failure) = answered {
        tracing::debug!(%failure, "a request went before the browser's interception answered it");
    }
}

/// Whether the worker whose target is `worker`, which a script fetch names
/// as its frame, is of a context that blocks its workers; one the browser
/// cannot name a context for is held as blocking, so a failed lookup lets
/// no worker run (ADR 0014: the policy fails closed).
async fn blocks(browser: &Browser, worker: &str) -> bool {
    let Ok(found) = browser
        .try_send(
            None,
            &target::GetTargetInfoParams {
                target_id: Some(worker.to_owned()),
            },
        )
        .await
    else {
        return true;
    };
    matches!(
        browser
            .service_workers
            .of(found.target_info.browser_context_id.as_ref()),
        Some(ServiceWorkers::Block)
    )
}

#[cfg(test)]
mod tests {
    use super::{ServiceWorkerChoices, ServiceWorkers, browser, lock};

    /// A context's choice goes once the context closes, with what it holds
    /// of its Session's Run, and another context's stays.
    #[test]
    fn a_closed_contexts_choice_goes() {
        let choices = ServiceWorkerChoices::default();
        let (closed, open) = (
            browser::BrowserContextID::from("closed"),
            browser::BrowserContextID::from("open"),
        );
        for context in [&closed, &open] {
            lock(&choices.contexts).insert(context.clone(), ServiceWorkers::Block);
        }

        choices.forget(&closed);

        assert_eq!(
            (
                choices.of(Some(&closed)).is_some(),
                choices.of(Some(&open)).is_some()
            ),
            (false, true)
        );
    }
}
