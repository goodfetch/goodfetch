//! A Session's pages (ADR 0085; pitfall B15): every page as it opens, one
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

use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_cdp::generated::target;
use goodfetch_engine::events::Events;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Values;
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::connection::ClosedValue;
use goodfetch_protocol::governor::WorkKind;
use goodfetch_protocol::page::events::PageEventName;
use goodfetch_protocol::page::session_pages::{
    PAGE_CLOSE, PAGE_POPUP, PagePopup, SESSION_PAGE, SessionOnEnded, SessionPages,
};
use goodfetch_protocol::page::{PageId, PageOpened, SessionId};
use goodfetch_protocol::settings::page_check::PageCheck;
use tokio::sync::watch;

use super::life::{Ending, PageLife, session_closed};
use super::lifecycle::PageEffect;
use super::{OpenPage, Session, Sessions, action_timeout, wait_deadline};
use crate::driver::{OnChild, OnPopup, PageDriver, PopupHandler, SessionEgress};
use crate::hiding::{PageHiding, Sheets};
use crate::page_events::{PageEvents, Refusals};
use crate::page_functions::FrameScripts;
use crate::page_value::{Observer, Reader};
use crate::pool::InBrowser;
use crate::recovery::{Checking, Declared};

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
    pub(super) observer: Option<Observer>,
    /// Its own settings: none for a popup, which takes its Session's.
    pub(super) settings: Values,
}

/// A page its Session is taking in, until it is kept: if its taking in
/// fails, or its future is dropped (a tier race's navigation that lost
/// while its page opened), its events, dialogs and interception are taken
/// out and its target closes, as an [`EnginePage`](super::EnginePage)'s
/// does. Each of those parts holds the page's driver, and so its browser
/// and its Run: one left behind would keep them past the Run's end, though
/// the end no longer waits for it (#1763).
struct TakenIn<'a> {
    sessions: &'a Sessions,
    page: PageId,
    driver: PageDriver,
    run: Run,
    /// Its place in its browser, until it is kept.
    in_browser: Option<InBrowser>,
}

impl TakenIn<'_> {
    /// The page goes into its Session's table with nothing awaited first,
    /// where its close, its Session's end or its Run's takes it out:
    /// answers its place in its browser.
    fn kept(mut self) -> InBrowser {
        self.in_browser
            .take()
            .expect("invariant: a page being taken in holds its place until it is kept")
    }
}

impl Drop for TakenIn<'_> {
    fn drop(&mut self) {
        let Some(in_browser) = self.in_browser.take() else {
            return;
        };
        let pages = std::slice::from_ref(&self.page);
        self.sessions.events.close(pages);
        self.sessions.dialogs.close(pages);
        self.sessions.interception.close_pages(pages);
        let driver = self.driver.clone();
        self.run.spawn(async move {
            driver.close().await;
            drop(in_browser);
        });
    }
}

/// How many calls listen to each Session's `page` event, each counted when
/// its call arrives, so a page published while the call waits for its
/// Session is not missed. Clones share them.
#[derive(Clone, Default)]
pub(super) struct PageListeners(
    #[expect(
        clippy::disallowed_types,
        reason = "one map read and changed alone, never with another lock or across an `.await`, so there is no interleaving to test under loom (ADR 0060)"
    )]
    Arc<std::sync::Mutex<BTreeMap<SessionId, usize>>>,
);

impl PageListeners {
    fn counts(&self) -> std::sync::MutexGuard<'_, BTreeMap<SessionId, usize>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether any call listens to the Session `session`'s `page` event.
    fn any(&self, session: &SessionId) -> bool {
        self.counts().contains_key(session)
    }

    /// Counts one listener of the Session `session`'s `page` event while
    /// the answer lives.
    fn listen(&self, session: &SessionId) -> Listening {
        *self.counts().entry(session.clone()).or_default() += 1;
        Listening {
            listeners: self.clone(),
            session: session.clone(),
        }
    }
}

/// On a Borrowed browser, the main frames of every page a Session held:
/// only their popups are the Session's, since the person's own windows share
/// its browser context (ADR 0056).
#[derive(Clone, Default)]
pub(super) struct HeldFrames(Arc<tokio::sync::Mutex<BTreeSet<target::TargetID>>>);

/// What a Session does with the page targets its browser context's pages
/// open: in a browser context of its own, each is its popup; on a Borrowed
/// browser, only one a page it held opened.
struct SessionPopups {
    sessions: Sessions,
    session: SessionId,
    /// On a Borrowed browser, the frames it held.
    held: Option<HeldFrames>,
}

impl PopupHandler for SessionPopups {
    fn holds(&self, opener: target::TargetID) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let held = self.held.clone();
        Box::pin(async move {
            match held {
                Some(held) => held.0.lock().await.contains(&opener),
                None => true,
            }
        })
    }

    fn take_in(
        &self,
        driver: PageDriver,
        opener: target::TargetID,
    ) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let (sessions, session) = (self.sessions.clone(), self.session.clone());
        Box::pin(async move { sessions.popup(&session, driver, &opener).await })
    }
}

/// One listener of a Session's `page` event, counted while it lives.
pub(crate) struct Listening {
    listeners: PageListeners,
    session: SessionId,
}

impl Drop for Listening {
    fn drop(&mut self) {
        let mut counts = self.listeners.counts();
        if let Some(count) = counts.get_mut(&self.session) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&self.session);
            }
        }
    }
}

impl Sessions {
    /// Takes in the page `adopting` describes as one of its Session's: it is
    /// named, followed and announced, and its Session's popups handed back
    /// to it; `closed` when its Session closed meanwhile.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future before the page is in its Session's table
    /// leaves none of its parts behind, and closes its target.
    pub(super) async fn adopt(&self, adopting: Adopting) -> Result<PageId, CallError> {
        let Adopting {
            open,
            driver,
            in_browser,
            admission,
            events: stream,
            observer,
            settings,
        } = adopting;
        let run = open.run.clone();
        let session = &super::session_id(run.id(), open.position);
        let action_timeout = action_timeout(&run);
        let page = {
            let mut state = self.state.lock().await;
            state.opened_pages += 1;
            PageId(format!("page-{}", state.opened_pages))
        };
        let browser = in_browser.browser();
        let taken_in = TakenIn {
            sessions: self,
            page: page.clone(),
            driver: driver.clone(),
            run: run.clone(),
            in_browser: Some(in_browser),
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
            driver.events(),
            wait_deadline(&run),
            observer,
        );
        let frame_scripts = FrameScripts::default();
        // The page's events read each out-of-process frame's network, and
        // each frame takes the page's init scripts, before it runs (H7, M20).
        let in_frames = vec![events.frames(), frame_scripts.frames(&driver)];
        self.events.insert(page.clone(), events);
        self.dialogs
            .start(page.clone(), driver.clone(), &run, driver.events());
        let (hiding, sheets, routed) = self
            .hold(
                (&page, session),
                &driver,
                &run,
                &open.settings,
                (refusals, in_frames),
            )
            .await?;
        let mut state = self.state.lock().await;
        if !state.sessions.contains_key(session) {
            drop(state);
            return Err(session_closed(session));
        }
        let life = PageLife::new(page.clone(), driver.value().lease().cloned(), routed);
        let page_check = open.checking(driver.value().page_check());
        // Nothing awaits from here until the page is in the table.
        let in_browser = taken_in.kept();
        let opened = OpenPage {
            clock: run.clock(),
            run,
            session: session.clone(),
            settings,
            driver,
            life,
            action_timeout,
            hiding,
            sheets,
            page_check,
            frame_scripts,
            injections: open.injections.clone(),
            _admission: Arc::new(admission),
            in_browser: Arc::new(in_browser),
        };
        state.pages.insert(page.clone(), opened.clone());
        drop(state);
        // Its popups are followed once it is in the table, so the engine's
        // Sessions lock is never held across the Session's frames' lock
        // (STYLE.md); a popup it opens finds its opener there.
        self.follow_popups(session, &open, &opened.driver).await;
        self.watch(page.clone(), opened, Arc::clone(&open), browser);
        if self.page_listeners.any(session) {
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

    /// Puts the page `page` of the Session `session`, which `driver` drives
    /// with the Session's own `settings`, under the interception, live
    /// hiding and the Consent prompt runner, each of its out-of-process
    /// frames and dedicated workers held by them, and then by `in_frames`,
    /// before it runs; answers its live hiding, the documents' hiding
    /// sheets it shares with the runner, and how many of its requests wait
    /// on a route handler now.
    async fn hold(
        &self,
        (page, session): (&PageId, &SessionId),
        driver: &PageDriver,
        run: &Run,
        settings: &Values,
        (refusals, in_frames): (Refusals, Vec<OnChild>),
    ) -> Result<(Option<Arc<PageHiding>>, Arc<Sheets>, watch::Receiver<usize>), CallError> {
        let held = self
            .interception
            .start(page, session, driver, run, settings, refusals)
            .await?;
        // Live hiding and the Consent prompt runner share each document's
        // one hiding sheet.
        let sheets = Arc::new(Sheets::new(driver.clone(), run));
        let hiding = self
            .hiding
            .start(page, session, driver, run, &held.clean_up, &sheets)
            .await?;
        let consent = self.consent.start(page, driver, run, &sheets).await?;
        // Each out-of-process frame and dedicated worker waits until the
        // interception holds it, the runner prepared it and the page's
        // other parts took it in.
        let mut handlers = vec![held.children, consent];
        handlers.extend(in_frames);
        driver.follow_children(run, handlers).await?;
        Ok((hiding, sheets, held.routed))
    }

    /// Hands the page targets the pages of `driver`'s browser context open
    /// to the Session `session`, `open`, which on a Borrowed browser now
    /// holds `driver`'s main frame too.
    async fn follow_popups(&self, session: &SessionId, open: &Session, driver: &PageDriver) {
        if let Some(held) = &open.held_frames {
            held.0.lock().await.insert(driver.main_frame().clone());
        }
        let popups: OnPopup = Arc::new(SessionPopups {
            sessions: self.clone(),
            session: session.clone(),
            held: open.held_frames.clone(),
        });
        driver.follow_popups(popups);
    }

    /// Takes in `driver`'s popup, which the page whose target is `opener`
    /// opened in the Session `session`, as a page of the Session, and
    /// announces it on its opener: whether it did. A popup of a page the
    /// engine no longer holds, or of a Session gone, is not taken in.
    async fn popup(
        &self,
        session: &SessionId,
        driver: PageDriver,
        opener: &target::TargetID,
    ) -> bool {
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
            return false;
        };
        let Ok(opener_events) = self.events.get(&opener_id) else {
            return false;
        };
        let run = open.run.clone();
        let admission = run.governor().admit(WorkKind::BrowserPage).await;
        let in_browser = open.seat.popup_in(opener.in_browser.browser(), &driver);
        let driver = driver
            .for_session(SessionEgress {
                proxy: open.proxy.clone(),
                relay: open.relay.clone(),
            })
            .with_value(opener.driver.value().clone(), run.clock())
            .with_reader(opener.driver.reader().map(Reader::for_page));
        let adopted = self
            .adopt(Adopting {
                open,
                driver,
                in_browser,
                admission,
                events: opener_events.stream().clone(),
                observer: None,
                settings: Values::default(),
            })
            .await;
        match adopted {
            Ok(popup) => {
                opener_events.publish(
                    PageEventName::Popup,
                    &PAGE_POPUP,
                    &PagePopup {
                        page_id: opener_id,
                        popup: PageOpened {
                            page_id: popup,
                            session_id: session.clone(),
                        },
                    },
                );
                true
            }
            Err(error) => {
                tracing::warn!(%error, "a popup was not taken in");
                false
            }
        }
    }

    /// The open pages of the Session `session`, in the order they opened;
    /// `closed` when it is closed.
    pub(crate) async fn pages(
        &self,
        session: &SessionId,
        context: &CallContext,
    ) -> Result<SessionPages, CallError> {
        // A crashed Session raises `page_crashed` again, and a closing one
        // `closed` (S21, R).
        self.session(session, context).await?.life.call().await?;
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

    /// Counts a listener of the Session `session`'s `page` event from now,
    /// when its call arrives, until the answer drops.
    pub(crate) fn listen_for_pages(&self, session: &SessionId) -> Listening {
        self.page_listeners.listen(session)
    }

    /// Listens to the Session `session`'s `page` event, as `listening`
    /// counts, until it closes or its Run ends; `closed` when it is closed.
    pub(crate) async fn listen_to_pages(
        &self,
        session: &SessionId,
        listening: Listening,
        context: &CallContext,
    ) -> Result<SessionOnEnded, CallError> {
        let open = self.session(session, context).await?;
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
    /// closing its window does, its `beforeunload` handlers running first, and
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
            // Its target's end is then the program's close (P8, P29).
            page.life.ask_to_close();
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
                self.page_ended(page_id, &page.run, &page.life, effect)
                    .await;
            }
            effect => {
                self.page_ended(page_id, &page.run, &page.life, effect)
                    .await;
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

impl Session {
    /// How its pages are judged: by `page_check`, the one a page's value
    /// carries from its crawl request's Label, when given (ADR 0093), else
    /// by its Page check, when it declares one; and whether it may hold a
    /// login, having opened from a Session source. Only a Borrowed
    /// browser's Session does today, at a `cdp_url`; a fresh one holds none
    /// of its own until a `login` or a Saved identity can give it one (ADR
    /// 0006).
    fn checking(&self, page_check: Option<&PageCheck>) -> Arc<Checking> {
        Arc::new(Checking {
            declared: page_check.map_or_else(
                || Declared::of(&self.run, &self.settings),
                |check| Some(Declared::label(check)),
            ),
            holds_login: self.held_frames.is_some(),
        })
    }
}
