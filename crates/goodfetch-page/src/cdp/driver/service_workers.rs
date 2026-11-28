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
//! `Service-Worker: script` header and the worker it loads. The same
//! interception has a let-through fetch send the user agent and client
//! hints its context shows, which reach no request the browser sends itself.
//!
//! A site's shared worker, which belongs to no page either, is attached the
//! same way and handed to its context's Session, whose interception holds
//! its requests before it runs, whatever the context chose for its service
//! workers (ADR 0014).
//!
//! Any other worker (a browser extension's, or one of a context no Session
//! named) is let run untouched, but for a headless browser's identity,
//! which every worker takes before it runs, as every page does (`identity`).
//! A worker of a context whose Session's Fingerprint method shows other
//! values than the machine's then takes those a worker reads (`masking`),
//! still before it runs.
#![expect(
    clippy::disallowed_types,
    reason = "each context's choice is set once and read by the browser's one event reader, each lock held for one map operation, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use goodfetch_cdp::generated::{browser, emulation, fetch, network, runtime, target};
use goodfetch_cdp::{Failure, RawEvent, SessionId as TargetSession, Subscription, SubscriptionEnd};
use goodfetch_engine::CallError;
use goodfetch_engine::run_identity::RunIdentity;
use tokio::sync::{OnceCell, watch};
use tokio_util::sync::CancellationToken;

use super::{Browser, PageDriver};
use crate::cdp::{CdpPage, Emulated};

/// The target type of a service worker.
const SERVICE_WORKER: &str = "service_worker";

/// The target type of a shared worker.
const SHARED_WORKER: &str = "shared_worker";

/// The header the browser's fetch of a worker's script carries.
const SCRIPT_HEADER: &str = "service-worker";

/// What one service or shared worker's Session does once the browser
/// attached the engine to it, before it runs: given the worker's driver and
/// the token cancelled once the worker's target goes, it answers whether
/// the interception now holds the worker's requests, whose tasks start on
/// the identity it is given, its browser's owner's (ADR 0146). A worker it
/// does not hold never runs: the domain policy fails closed (ADR 0014).
pub(crate) type OnWorker = Arc<
    dyn Fn(PageDriver, CancellationToken, RunIdentity) -> Pin<Box<dyn Future<Output = bool> + Send>>
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
    /// The values each context's service and shared workers show in place
    /// of the machine's, when its Session's Fingerprint method has any.
    masks: Mutex<BTreeMap<browser::BrowserContextID, Emulated>>,
    /// Each attached worker's token, by its session, cancelled once it goes.
    attached: Mutex<BTreeMap<TargetSession, CancellationToken>>,
    /// The browser's own interception of workers' scripts, once a context
    /// blocks its workers or shows another user agent than the browser's.
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

    /// The values the workers of the context `context` show, if its
    /// Session's Fingerprint method has any.
    fn mask_of(&self, context: Option<&browser::BrowserContextID>) -> Option<Emulated> {
        lock(&self.masks).get(context?).cloned()
    }

    /// Lets go of the choices of the context `context`, which closed, with
    /// what they hold of its Session's Run.
    pub(super) fn forget(&self, context: &browser::BrowserContextID) {
        let forgotten = lock(&self.contexts).remove(context);
        let shared = lock(&self.shared).remove(context);
        let masks = lock(&self.masks).remove(context);
        drop((forgotten, shared, masks));
    }
}

impl PageDriver {
    /// The driver of a service worker of the browser context `context`,
    /// which the browser attached on `session` and whose target is
    /// `worker`: only its Session's interception sends commands on it, and
    /// no navigation, gesture or document is its. Its Session's proxy and
    /// relay are the ones [`Self::with_egress`] gives it.
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
            other_code: Arc::new(watch::Sender::new(0)),
            spent: Duration::ZERO,
            value: super::RequestValue::new(crate::page_value::PageValue::empty()),
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
    /// context changes nothing. `block`, or a context that shows another
    /// user agent than the browser's own, turns on the browser's own
    /// interception of workers' scripts first, so their fetch can fail or
    /// send the context's.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses that
    /// interception.
    pub(crate) async fn service_workers(&self, choice: ServiceWorkers) -> Result<(), CallError> {
        let browser = &self.browser;
        if matches!(choice, ServiceWorkers::Block)
            || browser
                .state
                .service_workers
                .shown(browser.identity.as_ref(), Some(&self.context))
                .is_some()
        {
            browser.enable_worker_scripts(&browser.opening()).await?;
        }
        lock(&browser.state.service_workers.contexts)
            .entry(self.context.clone())
            .or_insert(choice);
        Ok(())
    }

    /// Hands, once for the page's browser context, the shared workers of
    /// its sites to `handler` before they run; a later handler for the same
    /// context changes nothing.
    pub(crate) fn shared_workers(&self, handler: OnWorker) {
        lock(&self.browser.state.service_workers.shared)
            .entry(self.context.clone())
            .or_insert(handler);
    }

    /// Has, once for the page's browser context, each of its service and
    /// shared workers show those of `values` a worker reads before it runs;
    /// later values for the same context change nothing.
    pub(crate) fn emulate_in_context_workers(&self, values: Emulated) {
        lock(&self.browser.state.service_workers.masks)
            .entry(self.context.clone())
            .or_insert(values);
    }
}

impl Browser {
    /// Turns on, once per browser, the browser's own interception of workers'
    /// scripts (`Other` requests at request stage), so a worker's script fetch
    /// can fail when blocked or send its context's identity instead of the
    /// headless browser's own.
    async fn enable_worker_scripts(&self, call: &crate::errors::Call) -> Result<(), CallError> {
        self.state
            .service_workers
            .scripts
            .get_or_try_init(|| async {
                self.send(
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
                    call,
                )
                .await
                .map(drop)
            })
            .await?;
        Ok(())
    }
}

/// Starts following `browser`'s service workers and new pages on `tasks`,
/// its owner's (ADR 0146): its browser events are read from now, then the
/// browser attaches the engine to each worker and each page before it runs
/// (`popups`), each such task one of its owner's. A headless browser (whose
/// pages show a non-headless window identity) also turns on worker-script
/// interception here at launch, once per browser rather than on its first
/// page's opening (C7, ADR 0140).
///
/// # Errors
///
/// The code of opening a page when the browser refuses to attach or
/// intercept worker scripts.
pub(super) async fn start(browser: &Browser, tasks: &RunIdentity) -> Result<(), CallError> {
    let events = browser.connection.subscribe(None);
    tasks.spawn(follow(events, browser.clone(), tasks.clone()));
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
        .map(drop)?;
    if browser.identity.is_some() {
        browser.enable_worker_scripts(&browser.launching()).await?;
    }
    Ok(())
}

/// Reads the browser's own events until its connection ends: each worker
/// attached is handed on as its context chose, each one gone is let go,
/// and each script fetch the browser's interception paused is failed or
/// sent on. A reader that fell 4,096 events behind lost what the browser
/// told meanwhile, so a worker or a page it attached then may wait for the
/// engine forever: the browser is marked so, which takes a launched one
/// out of service, and its events are read again from then on, so what it
/// attaches next is served. No Session is crashed and no crash is charged:
/// the browser runs on (spec #2256, #3261).
async fn follow(mut events: Subscription, browser: Browser, tasks: RunIdentity) {
    loop {
        while let Some(event) = events.next().await {
            take_in(&event, &browser, &tasks);
        }
        if events.ended() != Some(SubscriptionEnd::FellBehind) {
            return;
        }
        tracing::warn!(
            executable = %browser.executable,
            "the reader of a browser's own events fell behind and lost some: it reads them again, and what the browser attached meanwhile may never run"
        );
        browser.root_fell_behind.cancel();
        events = browser.connection.subscribe(None);
    }
}

/// Acts on `event`, one of the browser's own, starting what it calls for on
/// `tasks`, the browser's owner.
fn take_in(event: &RawEvent, browser: &Browser, tasks: &RunIdentity) {
    if let Some(attached) = event.read::<target::AttachedToTargetEvent>() {
        if attached.target_info.r#type == SERVICE_WORKER {
            tasks.spawn(attach(browser.clone(), attached, tasks.clone()));
        } else if attached.target_info.r#type == SHARED_WORKER {
            let choice = browser
                .state
                .service_workers
                .shared_of(attached.target_info.browser_context_id.as_ref())
                .map(ServiceWorkers::Allow);
            tasks.spawn(hold_then_run(
                browser.clone(),
                attached,
                tasks.clone(),
                choice,
            ));
        } else if attached.target_info.r#type == super::popups::PAGE {
            tasks.spawn(super::popups::attach(browser.clone(), attached));
        }
    } else if let Some(detached) = event.read::<target::DetachedFromTargetEvent>() {
        let detached = TargetSession(detached.session_id);
        browser.state.popups.forget(&detached);
        if let Some(gone) = lock(&browser.state.service_workers.attached).remove(&detached) {
            gone.cancel();
        }
    } else if let Some(paused) = event.read::<fetch::RequestPausedEvent>() {
        tasks.spawn(script_fetch(browser.clone(), paused));
    }
}

/// The service worker `attached` waits for the engine: under `allow` its
/// Session's interception takes it first, under `block` it is closed; then
/// it runs.
async fn attach(browser: Browser, attached: target::AttachedToTargetEvent, tasks: RunIdentity) {
    let choice = browser
        .state
        .service_workers
        .of(attached.target_info.browser_context_id.as_ref());
    hold_then_run(browser, attached, tasks, choice).await;
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
    tasks: RunIdentity,
    choice: Option<ServiceWorkers>,
) {
    let session = TargetSession(attached.session_id);
    let info = attached.target_info;
    let of_a_site = info.url.starts_with("http://") || info.url.starts_with("https://");
    let choice = choice.filter(|_| of_a_site);
    let held = match (choice, info.browser_context_id.clone()) {
        (Some(ServiceWorkers::Allow(handle)), Some(context)) => {
            let gone = tasks.owner().child_token();
            lock(&browser.state.service_workers.attached).insert(session.clone(), gone.clone());
            let driver = PageDriver::service_worker(
                browser.clone(),
                session.clone(),
                context,
                info.target_id.clone(),
            );
            tasks.spawn(driver.reading(None));
            handle(driver, gone, tasks).await
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
    let context = info.browser_context_id;
    if let Some(mask) = browser.state.service_workers.mask_of(context.as_ref())
        && let Some(context) = context
    {
        let driver =
            PageDriver::service_worker(browser.clone(), session.clone(), context, info.target_id);
        if let Err(error) = CdpPage::of(&driver).emulate_worker_opening(&mask).await {
            tracing::debug!(%error, "a worker went before it took its Session's Fingerprint method's values");
        }
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
/// failure does; any other goes on, a worker's script fetch with the user
/// agent and client hints its context shows (`shown`).
async fn script_fetch(browser: Browser, paused: fetch::RequestPausedEvent) {
    let script = paused.request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case(SCRIPT_HEADER) && value.as_str() == Some("script")
    });
    let worker = if script {
        Some(context_of(&browser, &paused.frame_id).await)
    } else {
        None
    };
    // A worker the browser cannot name a context for is held as blocking,
    // so a failed lookup lets no worker run (ADR 0014: the policy fails
    // closed).
    let blocked = match &worker {
        None => false,
        Some(Err(_)) => true,
        Some(Ok(context)) => matches!(
            browser.state.service_workers.of(context.as_ref()),
            Some(ServiceWorkers::Block)
        ),
    };
    let headers = match worker {
        Some(Ok(context)) => browser
            .state
            .service_workers
            .shown(browser.identity.as_ref(), context.as_ref())
            .map(|identity| with_identity(&paused.request.headers, &identity)),
        None | Some(Err(_)) => None,
    };
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
                    headers,
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

/// The browser context of the worker whose target is `worker`, which a
/// script fetch names as its frame, as the browser names it.
async fn context_of(
    browser: &Browser,
    worker: &str,
) -> Result<Option<browser::BrowserContextID>, Failure> {
    browser
        .try_send(
            None,
            &target::GetTargetInfoParams {
                target_id: Some(worker.to_owned()),
            },
        )
        .await
        .map(|found| found.target_info.browser_context_id)
}

impl ServiceWorkerChoices {
    /// The user agent and client hints a worker of the context `context` shows:
    /// its Session's Fingerprint method's, or else its headless browser's
    /// identity; none with a window and no such method, where the browser's
    /// own are already right.
    fn shown(
        &self,
        identity: Option<&super::identity::Identity>,
        context: Option<&browser::BrowserContextID>,
    ) -> Option<Shown> {
        let masked = self.mask_of(context).and_then(|mask| mask.user_agent);
        match masked {
            Some(agent) => Some(Shown {
                hints: agent
                    .client_hints
                    .as_ref()
                    .map(crate::cdp::masking::metadata),
                user_agent: agent.user_agent,
                accept_language: agent.accept_language,
            }),
            None => identity.map(|identity| Shown {
                user_agent: identity.user_agent().to_owned(),
                accept_language: None,
                hints: Some(identity.hints().clone()),
            }),
        }
    }
}

/// The identity a worker's script fetch sends in place of the browser's
/// own.
struct Shown {
    /// The `User-Agent` header.
    user_agent: String,
    /// The `Accept-Language` header; none keeps the browser's.
    accept_language: Option<String>,
    /// The client hints; none for a user agent shown without them.
    hints: Option<emulation::UserAgentMetadata>,
}

/// `headers`, those of the fetch of a worker's script, with the identity
/// `shown` in place of the browser's own: Chromium fetches the script
/// outside every override a worker takes, and adds its own `User-Agent` and
/// `Accept-Language` after its interception unless they are set. A client
/// hint `shown` has no value for is left out, so no request carries one
/// the worker does not show.
fn with_identity(headers: &network::Headers, shown: &Shown) -> Vec<fetch::HeaderEntry> {
    let entry = |name: &str, value: String| fetch::HeaderEntry {
        name: name.to_owned(),
        value,
    };
    let mut sent: Vec<_> = headers
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.to_ascii_lowercase();
            let value = match lower.as_str() {
                "user-agent" => None,
                "accept-language" if shown.accept_language.is_some() => None,
                hint_name if hint_name.starts_with("sec-ch-ua") => shown
                    .hints
                    .as_ref()
                    .and_then(|hints| hint(hint_name, hints)),
                _ => value.as_str().map(str::to_owned),
            };
            value.map(|value| entry(name, value))
        })
        .collect();
    sent.push(entry("User-Agent", shown.user_agent.clone()));
    if let Some(languages) = &shown.accept_language {
        sent.push(entry("Accept-Language", languages.clone()));
    }
    sent
}

/// The value of the client hint header `name`, in lower case, that
/// `hints` show, as Chromium spells it; none for a hint they have no value
/// for.
fn hint(name: &str, hints: &emulation::UserAgentMetadata) -> Option<String> {
    let quoted = |value: &str| format!("\"{value}\"");
    let flag = |on: bool| if on { "?1" } else { "?0" }.to_owned();
    let brands = |list: &[emulation::UserAgentBrandVersion]| {
        list.iter()
            .map(|brand| format!("\"{}\";v=\"{}\"", brand.brand, brand.version))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match name {
        "sec-ch-ua" => hints.brands.as_deref().map(brands),
        "sec-ch-ua-full-version-list" => hints.full_version_list.as_deref().map(brands),
        "sec-ch-ua-mobile" => Some(flag(hints.mobile)),
        "sec-ch-ua-platform" => Some(quoted(&hints.platform)),
        "sec-ch-ua-platform-version" => Some(quoted(&hints.platform_version)),
        "sec-ch-ua-arch" => Some(quoted(&hints.architecture)),
        "sec-ch-ua-model" => Some(quoted(&hints.model)),
        "sec-ch-ua-full-version" => hints.full_version.as_deref().map(quoted),
        "sec-ch-ua-bitness" => hints.bitness.as_deref().map(quoted),
        "sec-ch-ua-wow64" => hints.wow64.map(flag),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        ServiceWorkerChoices, ServiceWorkers, Shown, browser, emulation, fetch, lock, network,
        with_identity,
    };

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

    /// The hints a Session shows: Edge 156 on Windows.
    fn hints() -> emulation::UserAgentMetadata {
        let brands = |version: &str| {
            Some(vec![
                emulation::UserAgentBrandVersion {
                    brand: "Microsoft Edge".to_owned(),
                    version: version.to_owned(),
                },
                emulation::UserAgentBrandVersion {
                    brand: "Chromium".to_owned(),
                    version: version.to_owned(),
                },
            ])
        };
        emulation::UserAgentMetadata {
            brands: brands("156"),
            full_version_list: brands("156.0.1.2"),
            full_version: Some("156.0.1.2".to_owned()),
            platform: "Windows".to_owned(),
            platform_version: "15.0.0".to_owned(),
            architecture: "x86".to_owned(),
            model: String::new(),
            mobile: false,
            bitness: Some("64".to_owned()),
            wow64: Some(false),
            form_factors: None,
        }
    }

    /// A headless build's script fetch, as the browser paused it.
    fn headless_fetch() -> network::Headers {
        serde_json::from_value(json!({
            "Service-Worker": "script",
            "Accept-Language": "en-US,en;q=0.9",
            "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/156.0.0.0",
            "sec-ch-ua": "\"HeadlessChrome\";v=\"156\"",
            "sec-ch-ua-mobile": "?0",
            "sec-ch-ua-platform": "\"Linux\"",
            "sec-ch-ua-full-version-list": "\"HeadlessChrome\";v=\"156.0.0.0\"",
            "sec-ch-ua-arch": "\"arm\"",
            "sec-ch-ua-form-factors": "\"Desktop\"",
        }))
        .expect("headers as the browser reports them")
    }

    /// The headers sent, by name.
    fn sent(headers: &[fetch::HeaderEntry]) -> Vec<(&str, &str)> {
        let mut sent: Vec<_> = headers
            .iter()
            .map(|header| (header.name.as_str(), header.value.as_str()))
            .collect();
        sent.sort_unstable();
        sent
    }

    /// The fetch of a worker's script sends the user agent, languages and
    /// client hints its Session shows, in place of the browser's own, and
    /// no hint it cannot show.
    #[test]
    fn a_script_fetch_sends_the_identity_its_session_shows() {
        let shown = Shown {
            user_agent: "Mozilla/5.0 (Windows NT 10.0) Edg/156".to_owned(),
            accept_language: Some("fr-FR,fr;q=0.9".to_owned()),
            hints: Some(hints()),
        };

        let headers = with_identity(&headless_fetch(), &shown);

        assert_eq!(
            sent(&headers),
            [
                ("Accept-Language", "fr-FR,fr;q=0.9"),
                ("Service-Worker", "script"),
                ("User-Agent", "Mozilla/5.0 (Windows NT 10.0) Edg/156"),
                (
                    "sec-ch-ua",
                    "\"Microsoft Edge\";v=\"156\", \"Chromium\";v=\"156\""
                ),
                ("sec-ch-ua-arch", "\"x86\""),
                (
                    "sec-ch-ua-full-version-list",
                    "\"Microsoft Edge\";v=\"156.0.1.2\", \"Chromium\";v=\"156.0.1.2\""
                ),
                ("sec-ch-ua-mobile", "?0"),
                ("sec-ch-ua-platform", "\"Windows\""),
            ]
        );
    }

    /// A user agent shown without client hints sends none, and one shown
    /// without languages keeps the browser's.
    #[test]
    fn a_user_agent_without_hints_sends_no_hint() {
        let shown = Shown {
            user_agent: "Mozilla/5.0 (Windows NT 10.0) Firefox/140".to_owned(),
            accept_language: None,
            hints: None,
        };

        let headers = with_identity(&headless_fetch(), &shown);

        assert_eq!(
            sent(&headers),
            [
                ("Accept-Language", "en-US,en;q=0.9"),
                ("Service-Worker", "script"),
                ("User-Agent", "Mozilla/5.0 (Windows NT 10.0) Firefox/140"),
            ]
        );
    }
}
