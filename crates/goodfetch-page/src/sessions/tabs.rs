//! A Session's tabs (ADR 0085; pitfall B15): every page as it opens, one
//! the program opened or a popup one of the Session's pages opened, taken
//! in the same way: its events and dialogs are read, the interception,
//! live hiding and the Consent prompt runner start on it, and the Session's
//! `page` event announces it, all before its first request leaves. A popup
//! takes its opener's value, so its navigations pass the opener's gate and
//! its commands its lease check (ADR 0087), and registers no response
//! observer, so nothing it loads is kept (ADR 0110); its opener emits
//! `popup`. `session.pages` lists the open pages, `session.on` listens to
//! the `page` event, and `page.close` closes a page.
//!
//! A popup waits for the governor's admission of one more browser page, as
//! a page the program opens does, paused in the browser meanwhile.

use std::sync::Arc;

use goodfetch_cdp::generated::target;
use goodfetch_engine::events::Events;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::connection::ClosedValue;
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::events::PageEventName;
use goodfetch_protocol::page::tabs::{
    PAGE_CLOSE, PAGE_POPUP, PagePopup, SESSION_PAGE, SessionOnEnded, SessionPages,
};
use goodfetch_protocol::page::{PageId, PageOpened, SessionId};

use super::life::{Ending, PageLife, session_closed};
use super::lifecycle::PageEffect;
use super::{OpenPage, Session, Sessions, timeouts, wait_deadline};
use crate::driver::{OnPopup, PageDriver};
use crate::hiding::Sheets;
use crate::page_events::{PageEvents, Refusals};
use crate::page_value::ResponseObserver;
use crate::pool::InBrowser;
use crate::recovery::Declared;

/// A page being taken in by its Session.
pub(super) struct Adopting {
    /// Its Session.
    pub(super) open: Arc<Session>,
    /// Its driver, carrying its value.
    pub(super) driver: PageDriver,
    /// Its place among its browser's open pages.
    pub(super) in_browser: InBrowser,
    /// Its admission of the governor's `browser_page` kind.
    pub(super) admission: Permit,
    /// The engine's event stream its events go on.
    pub(super) events: Events,
    /// Its response observer, when one is registered.
    pub(super) observer: Option<Arc<dyn ResponseObserver>>,
}

/// How many calls listen to a Session's `page` event.
#[derive(Default)]
pub(super) struct PageListeners(
    #[expect(
        clippy::disallowed_types,
        reason = "one counter read and changed alone, never with a lock or across an `.await`, so there is no interleaving to test under loom (ADR 0060)"
    )]
    std::sync::atomic::AtomicUsize,
);

impl PageListeners {
    /// Whether any call listens.
    fn any(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed) > 0
    }
}

/// The guard that counts one listener of a Session's `page` event while it
/// lives.
struct Listening(Arc<Session>);

impl Listening {
    fn new(session: Arc<Session>) -> Self {
        session
            .page_listeners
            .0
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(session)
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.0
            .page_listeners
            .0
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Sessions {
    /// Takes in the page `adopting` describes as one of its Session's: it is
    /// named, followed and announced, and its Session's popups handed back
    /// to it; `closed` when its Session closed meanwhile.
    pub(super) async fn adopt(&self, adopting: Adopting) -> Result<PageId, CallError> {
        let Adopting {
            open,
            driver,
            in_browser,
            admission,
            events: stream,
            observer,
        } = adopting;
        let run = open.run.clone();
        let session = &super::session_id(run.id(), open.position);
        let (_, action_timeout) = timeouts(&run);
        let page = {
            let mut state = self.state.lock().await;
            state.opened_pages += 1;
            PageId(format!("page-{}", state.opened_pages))
        };
        // The page's events are read from before its requests pause, so
        // every request of the page is followed.
        let refusals = Refusals::default();
        let events = PageEvents::start(
            page.clone(),
            driver.clone(),
            &run,
            stream.clone(),
            refusals.clone(),
            driver.events().await,
            wait_deadline(&run),
        );
        self.events.insert(page.clone(), events);
        self.dialogs
            .start(page.clone(), driver.clone(), &run, driver.events().await);
        let pages = std::slice::from_ref(&page);
        let started = async {
            let clean_up = self
                .interception
                .start(
                    &page,
                    session,
                    &driver,
                    &run,
                    &open.settings,
                    refusals,
                    observer,
                )
                .await?;
            // Live hiding and the Consent prompt runner share each
            // document's one hiding sheet.
            let sheets = Arc::new(Sheets::new(driver.clone()));
            let hiding = self
                .hiding
                .start(&page, session, &driver, &run, &clean_up, &sheets)
                .await?;
            self.consent.start(&page, &driver, &run, &sheets).await?;
            Ok::<_, CallError>(hiding)
        };
        let hiding = match started.await {
            Ok(hiding) => hiding,
            Err(error) => {
                self.events.close(pages);
                self.dialogs.close(pages);
                self.interception.close_pages(pages);
                return Err(error);
            }
        };
        let mut state = self.state.lock().await;
        if !state.sessions.contains_key(session) {
            drop(state);
            self.events.close(pages);
            self.dialogs.close(pages);
            self.interception.close_pages(pages);
            return Err(session_closed(session));
        }
        let browser = in_browser.browser();
        driver.follow_popups(self.popups_of(session));
        let opened = OpenPage {
            clock: run.clock(),
            run,
            session: session.clone(),
            driver,
            life: PageLife::new(page.clone()),
            action_timeout,
            hiding,
            page_check: Declared::of(&open.run, &open.settings).map(Arc::new),
            _admission: Arc::new(admission),
            in_browser: Arc::new(in_browser),
        };
        state.pages.insert(page.clone(), opened.clone());
        drop(state);
        self.watch(page.clone(), opened, Arc::clone(&open), browser);
        if open.page_listeners.any() {
            stream
                .publish(
                    &SESSION_PAGE,
                    Some(&session.0),
                    &PageOpened {
                        page_id: page.clone(),
                        session_id: session.clone(),
                    },
                )
                .await;
        }
        Ok(page)
    }

    /// What the Session `session` does with a popup one of its pages
    /// opened.
    fn popups_of(&self, session: &SessionId) -> OnPopup {
        let (sessions, session) = (self.clone(), session.clone());
        Arc::new(move |driver, opener| {
            let (sessions, session) = (sessions.clone(), session.clone());
            Box::pin(async move { sessions.popup(&session, driver, &opener).await })
        })
    }

    /// Takes in `driver`'s popup, which the page whose target is `opener`
    /// opened in the Session `session`, as a page of the Session, and
    /// announces it on its opener; a popup of a page the engine no longer
    /// holds is left alone.
    async fn popup(&self, session: &SessionId, driver: PageDriver, opener: &target::TargetID) {
        let found = {
            let state = self.state.lock().await;
            let open = state.sessions.get(session).cloned();
            let opener = state
                .pages
                .iter()
                .find(|(_, page)| &page.session == session && page.driver.main_frame() == opener)
                .map(|(page_id, page)| (page_id.clone(), page.clone()));
            open.zip(opener)
        };
        let Some((open, (opener_id, opener))) = found else {
            return;
        };
        let Ok(opener_events) = self.events.get(&opener_id) else {
            return;
        };
        let run = open.run.clone();
        let (navigation_timeout, _) = timeouts(&run);
        let admission = run.governor().admit(WorkKind::BrowserPage).await;
        let in_browser = open
            .seat
            .popup_in(opener.in_browser.browser(), &driver)
            .await;
        let driver = driver
            .for_session(open.proxy.clone(), navigation_timeout)
            .with_value(opener.driver.value().clone(), run.clock());
        let adopted = self
            .adopt(Adopting {
                open,
                driver,
                in_browser,
                admission,
                events: opener_events.stream().clone(),
                observer: None,
            })
            .await;
        match adopted {
            Ok(popup) => {
                opener_events
                    .publish(
                        PageEventName::Popup,
                        &PAGE_POPUP,
                        &PagePopup {
                            page_id: opener_id,
                            popup: PageOpened {
                                page_id: popup,
                                session_id: session.clone(),
                            },
                        },
                    )
                    .await;
            }
            Err(error) => tracing::debug!(%error, "a popup was not taken in"),
        }
    }

    /// The open pages of the Session `session`, in the order they opened;
    /// `closed` when it is closed.
    pub(crate) async fn pages(
        &self,
        session: &SessionId,
        context: &CallContext,
    ) -> Result<SessionPages, CallError> {
        self.session(session, context).await?;
        let mut pages: Vec<PageId> = self
            .state
            .lock()
            .await
            .pages
            .iter()
            .filter(|(_, page)| &page.session == session)
            .map(|(page_id, _)| page_id.clone())
            .collect();
        pages.sort_by_key(opening_number);
        Ok(SessionPages { pages })
    }

    /// Listens to the Session `session`'s `page` event until it closes or
    /// its Run ends; `closed` when it is closed.
    pub(crate) async fn listen_to_pages(
        &self,
        session: &SessionId,
        context: &CallContext,
    ) -> Result<SessionOnEnded, CallError> {
        let open = self.session(session, context).await?;
        let listening = Listening::new(Arc::clone(&open));
        tokio::select! {
            () = open.life.ended() => {}
            () = open.run.cancellation().cancelled() => {}
        }
        drop(listening);
        Ok(SessionOnEnded {})
    }

    /// Closes the page `page_id`, a popup as any other: its calls in flight
    /// end, it emits `close` and leaves; one closed already stays as it is.
    /// With `run_before_unload`, the page is asked to close as a person
    /// closing its tab does, its `beforeunload` handlers running first, and
    /// it leaves once its target is destroyed.
    pub(crate) async fn close_page(
        &self,
        page_id: &PageId,
        run_before_unload: bool,
    ) -> Result<(), CallError> {
        let page = match self.page(page_id).await {
            Ok(page) => page,
            Err(error) => {
                return if self.opened(page_id).await {
                    Ok(())
                } else {
                    Err(error)
                };
            }
        };
        if run_before_unload {
            return page
                .driver
                .close_running_before_unload(PAGE_CLOSE.name)
                .await;
        }
        match page.life.end(&Ending::Closed(ClosedValue::Page))? {
            PageEffect::Close => {
                page.driver.close().await;
                // The close the engine sent is confirmed (P29).
                let effect = page.life.end(&Ending::Destroyed)?;
                self.page_ended(page_id, &page.run, effect).await;
            }
            effect => {
                self.page_ended(page_id, &page.run, effect).await;
            }
        }
        Ok(())
    }

    /// Whether `page_id` names a page this engine opened.
    async fn opened(&self, page_id: &PageId) -> bool {
        let opened = self.state.lock().await.opened_pages;
        opening_number(page_id).is_some_and(|number| (1..=opened).contains(&number))
    }
}

/// The number a page's ID gives it in opening order.
fn opening_number(page: &PageId) -> Option<u64> {
    page.0.strip_prefix("page-")?.parse().ok()
}
