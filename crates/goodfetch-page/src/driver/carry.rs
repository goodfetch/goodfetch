//! What a Session carries from a browser being recycled to the one that
//! replaces it (ADR 0040: a Session is not its browser): the cookies of its
//! browser context and the `localStorage` of every origin its pages'
//! documents came from. Each origin's storage is read, and written again,
//! in a page of the Session's context that commits an empty document of
//! that origin, answered by the engine itself through `Fetch`, so no request
//! reaches the site; the script runs in the guard's isolated world, never
//! through `Runtime.enable` (ADR 0019), and the page closes once done.
#![expect(
    clippy::disallowed_types,
    reason = "a Session's origins are added and listed one at a time, never across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use data_encoding::BASE64;
use goodfetch_cdp::generated::{browser, fetch, network, page, storage, target};
use goodfetch_cdp::{Failure, SessionId as TargetSession};
use goodfetch_cdp::{RawEvent, Subscription};
use goodfetch_engine::CallError;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::{Browser, Connect, SessionDriver, isolated_world, world};
use crate::errors::Call;

/// The origins whose documents a Session's pages loaded, which its pages'
/// watchers add to.
#[derive(Clone, Debug, Default)]
pub(crate) struct Origins(Arc<Mutex<Watched>>);

#[derive(Debug, Default)]
struct Watched {
    origins: BTreeSet<String>,
    /// How to ask each watcher still going whether its page's reader read
    /// what the connection handed it: [`Origins::caught_up`].
    watchers: Vec<mpsc::Sender<oneshot::Sender<()>>>,
}

/// What a page's reader adds the origins of its documents with, and answers
/// a capture's ask with once it read every event the connection handed it
/// (`page_reader`): a page's origins watcher.
#[derive(Debug)]
pub(crate) struct Watcher {
    origins: Origins,
    asked: mpsc::Receiver<oneshot::Sender<()>>,
}

impl Watcher {
    /// Adds the origin of the document `event` shows a frame of the page
    /// navigated to, when it does.
    pub(crate) fn read(&self, event: &RawEvent) {
        let Some(navigated) = event.read::<page::FrameNavigatedEvent>() else {
            return;
        };
        let origin = navigated.frame.security_origin;
        if origin.starts_with("http://") || origin.starts_with("https://") {
            self.origins.watched().origins.insert(origin);
        }
    }

    /// Waits for a capture's ask, and answers it: its reader asks only
    /// once it holds no event unread. Never answers once no capture can ask.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no ask.
    pub(crate) async fn asked(&mut self) {
        match self.asked.recv().await {
            Some(answer) => {
                let _ = answer.send(());
            }
            None => std::future::pending().await,
        }
    }
}

/// How many asks a watcher holds unread: its Session captures one at a time.
const WATCHER_ASKS: usize = 1;

impl Origins {
    fn watched(&self) -> MutexGuard<'_, Watched> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn list(&self) -> Vec<String> {
        self.watched().origins.iter().cloned().collect()
    }

    /// Waits until each page's reader read every event the connection
    /// handed it so far, so an origin the browser reported before an answer
    /// the caller already holds is listed, however far the reader's task
    /// lagged. A watcher that went has nothing more to add.
    async fn caught_up(&self) {
        let watchers = self.watched().watchers.clone();
        for watcher in watchers {
            let (answer, answered) = oneshot::channel();
            if watcher.send(answer).await.is_ok() {
                let _ = answered.await;
            }
        }
    }

    /// The watcher a page's reader adds the origin of every document of the
    /// page's session with, until it ends: its main frame's and its
    /// frames'. A capture waits for it from now on, whether or not its
    /// reader started reading.
    pub(crate) fn watcher(self) -> Watcher {
        let (asks, asked) = mpsc::channel(WATCHER_ASKS);
        {
            let mut watched = self.watched();
            watched.watchers.retain(|watcher| !watcher.is_closed());
            watched.watchers.push(asks);
        }
        Watcher {
            origins: self,
            asked,
        }
    }
}

/// A Session's cookies and storage, taken from one browser for the next.
#[derive(Clone, Debug, Default)]
pub(crate) struct Carried {
    cookies: Vec<network::CookieParam>,
    /// Each origin's `localStorage` items, in their order.
    storage: Vec<(String, Vec<(String, String)>)>,
}

/// The function that answers an origin's `localStorage` as its entries.
const READ_STORAGE: &str = "() => Object.entries(localStorage)";

/// The function that sets each of its entries in an origin's
/// `localStorage`.
const WRITE_STORAGE: &str = "(items) => { for (const [key, value] of items) localStorage.setItem(key, value); return true; }";

/// The empty document an origin's page commits.
const EMPTY_DOCUMENT: &str = "<!doctype html><title></title>";

/// The command that sets `cookie` again where it was.
fn cookie_param(cookie: network::Cookie) -> network::CookieParam {
    network::CookieParam {
        name: cookie.name,
        value: cookie.value,
        url: None,
        domain: Some(cookie.domain),
        path: Some(cookie.path),
        secure: Some(cookie.secure),
        http_only: Some(cookie.http_only),
        same_site: cookie.same_site,
        expires: (!cookie.session).then_some(cookie.expires),
        priority: Some(cookie.priority),
        source_scheme: Some(cookie.source_scheme),
        source_port: Some(cookie.source_port),
        partition_key: cookie.partition_key,
    }
}

impl<B: Connect> SessionDriver<B> {
    /// The Session's cookies and the `localStorage` of each of `origins`,
    /// as its browser context holds them now; nothing when it has none.
    pub(crate) async fn capture(&self, origins: &Origins) -> Result<Carried, CallError> {
        let Some(context) = self.context.get() else {
            return Ok(Carried::default());
        };
        let browser = self.browser.connect().await?;
        let call = browser.opening();
        let cookies = browser
            .send(
                None,
                &storage::GetCookiesParams {
                    browser_context_id: Some(context.clone()),
                },
                &call,
            )
            .await?
            .cookies
            .into_iter()
            .map(cookie_param)
            .collect();
        // The browser's answer came after every commit it reported before it,
        // on the one connection: its watchers have those events to read.
        origins.caught_up().await;
        let mut stored = Vec::new();
        for origin in origins.list() {
            let read = on_origin(&browser, context, &origin, READ_STORAGE, Value::Null).await?;
            let items: Vec<(String, String)> = serde_json::from_value(read).unwrap_or_default();
            if !items.is_empty() {
                stored.push((origin, items));
            }
        }
        Ok(Carried {
            cookies,
            storage: stored,
        })
    }

    /// Puts `carried` into the Session's browser context, made first when
    /// it has none.
    pub(crate) async fn restore(&self, carried: &Carried) -> Result<(), CallError> {
        let browser = self.browser.connect().await?;
        let context = self.context_in(&browser).await?;
        let call = browser.opening();
        if !carried.cookies.is_empty() {
            browser
                .send(
                    None,
                    &storage::SetCookiesParams {
                        cookies: carried.cookies.clone(),
                        browser_context_id: Some(context.clone()),
                    },
                    &call,
                )
                .await?;
        }
        for (origin, items) in &carried.storage {
            on_origin(&browser, &context, origin, WRITE_STORAGE, json!(items)).await?;
        }
        Ok(())
    }
}

/// Calls `function` with `argument` in the guard's world of a page of
/// `context` that holds an empty document of `origin`, and answers its
/// value; the page closes whatever happens.
async fn on_origin(
    browser: &Browser,
    context: &browser::BrowserContextID,
    origin: &str,
    function: &str,
    argument: Value,
) -> Result<Value, CallError> {
    let call = browser.opening();
    let created = browser
        .send(
            None,
            &target::CreateTargetParams {
                url: "about:blank".to_owned(),
                browser_context_id: Some(context.clone()),
                left: None,
                top: None,
                width: None,
                height: None,
                window_state: None,
                enable_begin_frame_control: None,
                new_window: None,
                background: Some(true),
                for_tab: None,
                hidden: None,
                focus: None,
            },
            &call,
        )
        .await?;
    let answered = run_on(
        browser,
        &created.target_id,
        origin,
        (function, argument),
        &call,
    )
    .await;
    if let Err(failure) = browser
        .try_send(
            None,
            &target::CloseTargetParams {
                target_id: created.target_id,
            },
        )
        .await
    {
        tracing::debug!(%failure, "a Session's storage page was gone already");
    }
    answered
}

/// Attaches to the page `page_target`, commits an empty document of
/// `origin` in it and calls the function there.
async fn run_on(
    browser: &Browser,
    page_target: &target::TargetID,
    origin: &str,
    (function, argument): (&str, Value),
    call: &Call,
) -> Result<Value, CallError> {
    let session = browser.attach_own(page_target, call).await?;
    let mut events = browser.connection.subscribe(Some(&session));
    browser
        .send(
            Some(&session),
            &fetch::EnableParams {
                patterns: Some(vec![fetch::RequestPattern {
                    url_pattern: Some("*".to_owned()),
                    resource_type: None,
                    request_stage: Some(fetch::RequestStage::Request),
                }]),
                handle_auth_requests: None,
            },
            call,
        )
        .await?;
    browser
        .send(Some(&session), &page::EnableParams::default(), call)
        .await?;
    let committed = tokio::time::timeout(
        browser.deadlines.navigation.duration(),
        commit(browser, &session, &mut events, origin, call),
    )
    .await;
    // No later command waits on a subscription nobody reads.
    drop(events);
    match committed {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) | Err(_) => {
            tracing::warn!(%origin, "a Session's storage of this origin is not carried over");
            return Ok(Value::Null);
        }
        Ok(Err(error)) => return Err(error),
    }
    let world = browser
        .send(Some(&session), &isolated_world(page_target.clone()), call)
        .await?;
    let called = browser
        .send(
            Some(&session),
            &world::call(function, argument, world.execution_context_id),
            call,
        )
        .await?;
    Ok(called.result.value.unwrap_or(Value::Null))
}

/// Navigates the page of `session` to `origin`, answering each request it
/// sends with an empty document, until that document commits: `false`
/// when the browser refuses the navigation.
async fn commit(
    browser: &Browser,
    session: &TargetSession,
    events: &mut Subscription,
    origin: &str,
    call: &Call,
) -> Result<bool, CallError> {
    let params = page::NavigateParams {
        url: format!("{origin}/"),
        referrer: None,
        transition_type: None,
        frame_id: None,
        referrer_policy: None,
    };
    let mut navigate = std::pin::pin!(browser.send(Some(session), &params, call));
    let mut navigated = false;
    let mut committed = false;
    let mut reading = true;
    while !(navigated && committed) {
        // Both the answer and the commit are waited for, and a refused
        // navigation ends the wait whatever came before it, so the order
        // the two are polled in cannot change how it ends (#1770).
        tokio::select! {
            answered = &mut navigate, if !navigated => {
                if answered?.error_text.is_some() {
                    return Ok(false);
                }
                navigated = true;
            }
            event = events.next(), if reading => {
                let Some(event) = event else {
                    // A target the browser detached sends nothing more, and
                    // the navigation in flight on it failed with it.
                    if !navigated && super::detached(events) {
                        reading = false;
                        continue;
                    }
                    return Err(browser.failed(call, &Failure::Disconnected).await);
                };
                if let Some(paused) = event.read::<fetch::RequestPausedEvent>() {
                    let fulfilled = browser
                        .try_send(
                            Some(session),
                            &fetch::FulfillRequestParams {
                                request_id: paused.request_id,
                                response_code: 200,
                                response_headers: Some(vec![fetch::HeaderEntry {
                                    name: "content-type".to_owned(),
                                    value: "text/html; charset=utf-8".to_owned(),
                                }]),
                                binary_response_headers: None,
                                body: Some(BASE64.encode(EMPTY_DOCUMENT.as_bytes())),
                                response_phrase: None,
                            },
                        )
                        .await;
                    if let Err(failure) = fulfilled {
                        tracing::debug!(%failure, "a storage page's request went before it was answered");
                    }
                } else if let Some(frame) = event.read::<page::FrameNavigatedEvent>()
                    && frame.frame.parent_id.is_none()
                    && frame.frame.security_origin == origin
                {
                    committed = true;
                }
            }
            else => return Err(browser.failed(call, &Failure::Disconnected).await),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
