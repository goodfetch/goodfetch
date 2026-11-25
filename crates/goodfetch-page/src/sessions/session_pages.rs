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

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use goodfetch_cdp::generated::target;
use goodfetch_engine::events::Events;
use goodfetch_engine::governor::Permit;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::runs::{Run, identity, run_trace};
use goodfetch_engine::settings::Values;
use goodfetch_engine::tasks::TaskStarter;
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

use super::life::{Ending, PageLife};
use super::lifecycle::PageEffect;
use super::robots_txt;
use super::{
    OpenPage, Session, Sessions, State, action_timeout, events_run, interception_run, wait_deadline,
};
use crate::cdp::CdpPage;
use crate::consent;
use crate::consent_walls::Walls;
use crate::dialogs::PageDialogs;
use crate::driver::{OnPopup, PageDriver, PopupHandler, SessionEgress};
use crate::errors::page_closed;
use crate::gesture_order::{GestureOrder, PointerAsks};
use crate::hiding::{Hiding, PageHiding, Sheets};
use crate::init_scripts::InitScripts;
use crate::interception::{OnDocument, PageInterception};
use crate::page_children::{self, OnChild};
use crate::page_events::{self, PageEvents, Refusals};
use crate::page_owner::{BrowserPagePart, PageOwner, PageParts, RequestPart};
use crate::page_value::{Observer, PageValue, Reader};
use crate::page_worlds::PageWorlds;
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
struct TakenIn {
    page: PageId,
    driver: PageDriver,
    run: Run,
    /// Its Run's identity that starts its parts' tasks on its owner.
    tasks: RunIdentity,
    /// Its place in its browser, until it is kept.
    in_browser: Option<InBrowser>,
    /// Its events, once they are read.
    events: Option<PageEvents>,
    /// Its dialogs, once they are read.
    dialogs: Option<PageDialogs>,
    /// Its init scripts, once its frames take them.
    init_scripts: Option<InitScripts>,
    /// Its side of the interception, once the interception holds it.
    interception: Option<PageInterception>,
    /// The consent walls its Consent prompt runner leaves.
    walls: Walls,
    /// Its hiding, once live hiding and the Consent prompt runner started.
    hiding: Option<Hiding>,
}

impl TakenIn {
    /// The page goes into its Session's table with nothing awaited first,
    /// where its close, its Session's end or its Run's takes it out:
    /// answers its place in its browser, and its parts its owner holds.
    fn kept(mut self) -> (InBrowser, PageParts) {
        let in_browser = self
            .in_browser
            .take()
            .expect("invariant: a page being taken in holds its place until it is kept");
        let events = self
            .events
            .take()
            .expect("invariant: a page's events are read before it is held");
        let dialogs = self
            .dialogs
            .take()
            .expect("invariant: a page's dialogs are read before it is held");
        let init_scripts = self
            .init_scripts
            .take()
            .expect("invariant: a page's frames take its init scripts before it is held");
        let interception = self
            .interception
            .take()
            .expect("invariant: a page is kept once the interception holds it");
        let hiding = self
            .hiding
            .take()
            .expect("invariant: a page is kept once its hiding started");
        (
            in_browser,
            PageParts {
                browser: BrowserPagePart {
                    walls: self.walls.clone(),
                    hiding,
                    worlds: PageWorlds::default(),
                },
                request: RequestPart {
                    events,
                    dialogs,
                    init_scripts,
                    interception,
                },
                tasks: self.tasks.clone(),
            },
        )
    }
}

impl Drop for TakenIn {
    fn drop(&mut self) {
        let Some(in_browser) = self.in_browser.take() else {
            return;
        };
        if let Some(events) = &self.events {
            events.close();
        }
        if let Some(dialogs) = &self.dialogs {
            dialogs.close();
        }
        if let Some(interception) = &self.interception {
            interception.close();
        }
        let driver = self.driver.clone();
        self.run.spawn(async move {
            driver.close().await;
            drop(in_browser);
        });
    }
}

/// How many calls listen to one Session's `page` event, which the Session
/// holds, so the count goes with it (ADR 0146). A call is counted when it
/// arrives if its Session is open then, so a page published while the
/// call waits is not missed. Clones share it.
#[derive(Clone, Default)]
pub(super) struct PageListeners(
    #[expect(
        clippy::disallowed_types,
        reason = "one count, added to and read alone, never with another lock or value it must agree with, so there is no interleaving to test under loom (ADR 0060)"
    )]
    Arc<std::sync::atomic::AtomicUsize>,
);

impl PageListeners {
    /// Whether any call listens to the Session's `page` event.
    fn any(&self) -> bool {
        self.0.load(Ordering::Acquire) > 0
    }

    /// Counts one listener of the Session's `page` event while the answer
    /// lives.
    fn listen(&self) -> Listening {
        self.0.fetch_add(1, Ordering::AcqRel);
        Listening(Some(self.clone()))
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

/// One listener of a Session's `page` event, counted while it lives; not
/// yet counted when its Session was not open as its call arrived.
pub(crate) struct Listening(Option<PageListeners>);

impl Drop for Listening {
    fn drop(&mut self) {
        if let Some(listeners) = &self.0 {
            listeners.0.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Sessions {
    /// The ID of the next page the engine opens.
    async fn next_page_id(&self) -> PageId {
        next_page_id(&mut *self.state.lock().await)
    }

    /// The page `page`, which `driver` drives for `run` in its place
    /// `in_browser`, as it starts being taken in, its events going on
    /// `stream` and to `observer`: its events, read from before its requests
    /// pause so every request of the page is followed, its init scripts and
    /// its dialogs, each starting its tasks on `tasks`, the Run's identity
    /// that starts them on the page's owner; with the page's refusals and
    /// what each of its out-of-process frames takes before it runs: the
    /// page's events read the frame's network and the frame takes the
    /// page's init scripts (H7, M20).
    fn taking_in(
        &self,
        (page, driver, (run, tasks)): (&PageId, &PageDriver, (&Run, &RunIdentity)),
        in_browser: InBrowser,
        (stream, observer): (&Events, Option<Observer>),
    ) -> (TakenIn, Refusals, Vec<OnChild>) {
        let refusals = Refusals::default();
        let events = PageEvents::start(
            page.clone(),
            driver.clone(),
            page_events::RunPowers {
                identity: tasks.clone(),
                ..events_run(run)
            },
            stream.clone(),
            refusals.clone(),
            wait_deadline(run),
            observer,
        );
        let cdp = CdpPage::of(driver);
        let init_scripts = InitScripts::new(cdp.clone(), tasks.clone(), page.clone());
        let in_frames = vec![events.frames(), init_scripts.frames()];
        let taken_in = TakenIn {
            page: page.clone(),
            driver: driver.clone(),
            run: run.clone(),
            tasks: tasks.clone(),
            in_browser: Some(in_browser),
            events: Some(events),
            dialogs: Some(self.dialogs.start(page.clone(), cdp, tasks)),
            init_scripts: Some(init_scripts),
            interception: None,
            walls: Walls::default(),
            hiding: None,
        };
        (taken_in, refusals, in_frames)
    }

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
        let page = self.next_page_id().await;
        let browser = in_browser.browser();
        // The owner's tasks, which start before it does, its parts' among
        // them: their following ends with the page if the page is never
        // kept.
        let tasks = open.owner.child();
        let unkept = tasks.token().clone().drop_guard();
        let starting = identity(&run).starting_on(&tasks);
        let (mut taken_in, refusals, in_frames) = self.taking_in(
            (&page, &driver, (&run, &starting)),
            in_browser,
            (&stream, observer),
        );
        // The runner clicks on the page while it is taken in, before its
        // owner starts and lends it the pointer.
        let (order, pointer) = GestureOrder::new();
        let routed = self
            .hold(
                (&mut taken_in, session, (&tasks, &starting)),
                (&driver, open.as_ref()),
                (refusals, in_frames, pointer),
            )
            .await?;
        if !open.pages.open() {
            return Err(self.closed_session(session));
        }
        let life = PageLife::new(page.clone(), driver.value().lease().cloned(), routed);
        let page_check = open.checking(driver.value().page_check());
        // Nothing awaits from here until the page is in the table.
        let (in_browser, parts) = taken_in.kept();
        // Its owner, under its Session's (ADR 0146).
        let owner = PageOwner::start(
            (&open.owner, tasks),
            identity(&run).span(),
            (&page, parts, order),
            page_closed(&page),
            run.metrics(),
        );
        unkept.disarm();
        self.page_owners.kept(page.clone(), owner.clone());
        let opened = OpenPage {
            clock: run.clock(),
            run,
            session: session.clone(),
            settings,
            driver,
            owner,
            life,
            action_timeout,
            page_check,
            injections: open.injections.clone(),
            _admission: Arc::new(admission),
            in_browser: Arc::new(in_browser),
        };
        // Its Session holds it before the index names it: a Session that
        // ended meanwhile, or ends now, takes it with its own pages.
        if !open.pages.hold(page.clone(), opened.clone()) {
            self.page_owners.close(std::slice::from_ref(&page));
            return Err(self.closed_session(session));
        }
        if !self.index.kept(page.clone(), (session, &open)) {
            return Err(self.closed_session(session));
        }
        // Its popups are followed once it is in its Session, so no lock of
        // the Sessions is held across the Session's frames' lock
        // (STYLE.md); a popup it opens finds its opener there.
        self.follow_popups(session, &open, &opened.driver).await;
        self.watch(opened, Arc::clone(&open), browser);
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

    /// Puts the page `taken_in` takes in, of the Session `session`, `open`,
    /// which `driver` drives with the Session's own settings, under the
    /// interception, live hiding and the Consent prompt runner, each of its
    /// out-of-process frames and dedicated workers held by them, and then
    /// by `in_frames`, before it runs, followed on its owner's `tasks`, on
    /// which `starting`, the Run's identity its parts are given, starts
    /// theirs; `taken_in` then holds its side of
    /// the interception and its hiding, the documents' hiding sheets live
    /// hiding shares with the runner. Answers how many of its requests wait
    /// on a route handler now.
    async fn hold(
        &self,
        (taken_in, session, (tasks, starting)): (
            &mut TakenIn,
            &SessionId,
            (&TaskStarter, &RunIdentity),
        ),
        (driver, open): (&PageDriver, &Session),
        (refusals, in_frames, pointer): (Refusals, Vec<OnChild>, PointerAsks),
    ) -> Result<watch::Receiver<usize>, CallError> {
        let (run, settings) = (&open.run, &open.settings);
        let page = &taken_in.page.clone();
        // A page a program opened reads robots.txt for each document its
        // main frame receives, beside its navigation, never before it (ADR
        // 0086).
        let reader = robots_txt::Reader::of(self, page, session, &driver.value(), (run, starting))
            .map(|reader| Arc::new(reader) as Arc<dyn OnDocument>);
        let held = self
            .interception
            .start(
                page,
                &open.interception,
                driver,
                (&interception_run(run), starting),
                settings,
                (refusals, reader),
            )
            .await?;
        taken_in.interception = Some(held.page);
        // Live hiding and the Consent prompt runner share each document's
        // one hiding sheet.
        let sheets = Arc::new(Sheets::new(driver, starting));
        let live = PageHiding::start(
            (page, session),
            driver,
            (starting, run_trace(run)),
            &held.clean_up,
            &sheets,
            &open.hiding,
        )
        .await?;
        taken_in.hiding = Some(Hiding::new(live, Arc::clone(&sheets)));
        let consent = self
            .consent
            .start(
                page,
                (driver, pointer),
                consent::RunPowers {
                    identity: starting.clone(),
                    trace: run_trace(run).clone(),
                    run_index: open.run_index.clone(),
                },
                &sheets,
                &taken_in.walls,
            )
            .await?;
        // Each out-of-process frame and dedicated worker waits until the
        // interception holds it, the runner prepared it and the page's
        // other parts took it in.
        let mut handlers = vec![held.children, consent];
        handlers.extend(in_frames);
        page_children::follow(
            &CdpPage::of(driver),
            (tasks, identity(run).span()),
            handlers,
        )
        .await?;
        Ok(held.routed)
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
        let Some(open) = self.index.session(session) else {
            return false;
        };
        let Some((opener_id, opener)) = open.pages.find(|page| page.driver.main_frame() == opener)
        else {
            return false;
        };
        let Ok(opener_events) = self.page_owners.events(&opener_id) else {
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
            .with_value(PageValue::clone(&opener.driver.value()), run.clock())
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
        let open = self.session(session, context).await?;
        open.life.call().await?;
        let mut pages = open.pages.ids();
        pages.sort_by_key(opening_number);
        Ok(SessionPages { pages })
    }

    /// Counts a listener of the Session `session`'s `page` event from now,
    /// when its call arrives, until the answer drops, if the Session is
    /// open now; else [`Self::listen_to_pages`] counts it once it finds the
    /// Session.
    pub(crate) fn listen_for_pages(&self, session: &SessionId) -> Listening {
        self.index
            .session(session)
            .map_or(Listening(None), |open| open.page_listeners.listen())
    }

    /// Listens to the Session `session`'s `page` event, as `listening`
    /// counts, until it closes or its Run ends; `closed` when it is closed.
    pub(crate) async fn listen_to_pages(
        &self,
        session: &SessionId,
        mut listening: Listening,
        context: &CallContext,
    ) -> Result<SessionOnEnded, CallError> {
        let open = self.session(session, context).await?;
        if listening.0.is_none() {
            listening = open.page_listeners.listen();
        }
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
                self.page_ended(&page.run, &page.life, effect).await;
            }
            effect => self.page_ended(&page.run, &page.life, effect).await,
        }
        Ok(())
    }

    /// Replaces the request part of the open page `page_id` (ADR 0146's "A
    /// released Page"): the page serves its next request under a Page ID
    /// of its own, which this answers, and under `value`, that request's
    /// gate, lease check and Page check, and a call on `page_id` raises
    /// `closed` from then on. It sends the browser nothing. `closed` when
    /// the page is not `Open`: gone, ending, navigating or paused.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "#2182's release replaces a Page's request part with it"
        )
    )]
    pub(crate) async fn replace_request(
        &self,
        page_id: &PageId,
        value: PageValue,
    ) -> Result<PageId, CallError> {
        // Under the table's lock, under which a page's end reads its ID, so
        // that end finds the page by one ID or the other, never by neither.
        let mut state = self.state.lock().await;
        let Some(page) = self.index.page(page_id).filter(|page| page.life.open()) else {
            return Err(page_closed(page_id));
        };
        let Some(session) = self.index.session(&page.session) else {
            return Err(page_closed(page_id));
        };
        let next = next_page_id(&mut state);
        let owner = page.owner.for_request(&next);
        let renamed = OpenPage {
            owner: owner.clone(),
            page_check: session.checking(value.page_check()),
            ..page.clone()
        };
        if !self.index.renamed(page_id, next.clone(), renamed) {
            return Err(page_closed(page_id));
        }
        self.page_owners.renamed(page_id, next.clone(), owner);
        page.life.renamed(next.clone(), value.lease().cloned());
        // Every clone of the page's driver, its frames' and workers'
        // included, serves the next request under its value.
        page.driver.replace_value(value);
        Ok(next)
    }

    /// Whether `page_id` names a page this engine opened.
    async fn opened(&self, page_id: &PageId) -> bool {
        let opened = self.state.lock().await.opened_pages;
        opening_number(page_id).is_some_and(|number| (1..=opened).contains(&number))
    }
}

/// The next Page ID `state` gives, a page's the engine opens or a request's
/// a page serves next, in one order.
fn next_page_id(state: &mut State) -> PageId {
    state.opened_pages += 1;
    PageId(format!("page-{}", state.opened_pages))
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
