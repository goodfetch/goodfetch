//! How a page's end, and its browser's, reaches its Session and its program
//! (spec #594; ADRs 0040 and 0113): each page is followed until it ends
//! without the engine closing its browser, each browser of the Run's pool
//! until it is lost, and what their end did is acted on.
//!
//! A browser that quit or crashed leaves its Run's pool at once, and every
//! Session seated in it takes the loss, one that holds no page included:
//! a quit ends it, a crash crashes it (S8, S9). A crash counts once in
//! Usage, a renderer's for its page and a browser's for the browser,
//! however many pages it held (ADR 0040's note on #731, accepted
//! 2026-10-28). A page's `crash` goes out once its crash is counted, so a
//! program that read it reads the count in its Run's Usage: a browser's
//! crash, counted by the first of its Sessions and pages to take it, is
//! waited on by every other.

use std::sync::Arc;

use goodfetch_engine::runs::{Run, identity};
use goodfetch_engine::tasks::TaskStarter;
use goodfetch_protocol::connection::ENGINE_PANICKED;
use goodfetch_protocol::page::events::{PAGE_CLOSE, PAGE_CRASH, PAGE_EVENTS_LOST, PageEventName};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::usage::PAGE_CRASHES;
use tokio::sync::mpsc;
use tracing::Instrument as _;

use super::life::Ending;
use super::lifecycle::{PageEffect, SessionEffect, SessionEvent};
use super::{OpenPage, PageLife, Session, Sessions};
use crate::driver::{Lost, PageDriver};
use crate::errors::{run_stopped, session_panicked};
use crate::pool::BrowserLoss;

impl Sessions {
    /// Acts on what the end of the page `page_id`, whose state `life` is,
    /// did (`effect`): a crash emits `crash`, a renderer's once counted in
    /// Usage (ADR 0040), a browser's once its caller counted it; a page that
    /// closed emits `close`, then leaves with its events and routes, so later
    /// calls raise `closed`, or `page_memory_exceeded` again for one ended
    /// past its memory cap, `engine_panicked` again for one a panic of its
    /// owner's tasks closed and `page_events_lost` again for one whose
    /// reader fell behind; a page whose browser crashed leaves its browser,
    /// as a released one does.
    pub(super) async fn page_ended(
        &self,
        page_id: &PageId,
        run: &Run,
        life: &PageLife,
        effect: PageEffect,
    ) {
        let pages = std::slice::from_ref(page_id);
        match effect {
            PageEffect::RendererCrash | PageEffect::BrowserCrash => {
                if effect == PageEffect::RendererCrash {
                    charge_a_crash(run).await;
                }
                if let Some(owner) = self.page_owners.get(page_id) {
                    owner
                        .events()
                        .publish_end(PageEventName::Crash, &PAGE_CRASH);
                }
                if effect == PageEffect::BrowserCrash {
                    // Its browser is gone: nothing is left to release, and
                    // the page no longer holds the dead browser in its pool.
                    self.release(page_id, run).await;
                }
            }
            PageEffect::ClosedInBrowser
            | PageEffect::Confirm
            | PageEffect::CloseOvertaken
            | PageEffect::Quit => {
                if effect != PageEffect::Quit
                    && let Some(owner) = self.page_owners.get(page_id)
                {
                    owner
                        .events()
                        .publish_end(PageEventName::Close, &PAGE_CLOSE);
                }
                let mut state = self.state.lock().await;
                // One the Resource governor ended past its memory cap, or a
                // panic of one of its owner's tasks or its reader fallen
                // behind closed, goes into the ended pages before it leaves
                // its Session, so a call finds it in one or the other and
                // raises `page_memory_exceeded`, `engine_panicked` or
                // `page_events_lost` again until its Run stops; once the
                // Run's clean-up took it out, nothing keeps it.
                if life.keeps_its_end() && state.runs.contains_key(run.id()) {
                    state
                        .crashed
                        .insert(page_id.clone(), (run.id().clone(), life.clone()));
                }
                self.index.remove_page(page_id);
                drop(state);
                self.page_owners.close(pages);
            }
            // A crashed page releases what is left, its admission and its
            // place in its browser, and stays `Crashed`: its later calls
            // raise `page_crashed` again (P33).
            PageEffect::Release => self.release(page_id, run).await,
            PageEffect::Nothing
            | PageEffect::Serve
            | PageEffect::Wait
            | PageEffect::Commit
            | PageEffect::FailNavigation
            | PageEffect::Interrupt
            | PageEffect::Judge
            | PageEffect::Defer
            | PageEffect::Pause
            | PageEffect::EndPause
            | PageEffect::Close => {}
        }
    }

    /// The crashed page `page_id` of `run` gives back its admission and its
    /// place in its browser, its events, dialogs and routes, and stays
    /// `Crashed` for its later calls.
    async fn release(&self, page_id: &PageId, run: &Run) {
        let pages = std::slice::from_ref(page_id);
        let mut state = self.state.lock().await;
        // It goes into the ended pages before it leaves its Session.
        if let Some(page) = self.index.page(page_id) {
            state
                .crashed
                .insert(page_id.clone(), (run.id().clone(), page.life));
            self.index.remove_page(page_id);
        }
        drop(state);
        self.page_owners.close(pages);
    }

    /// The Session `open` takes the loss `lost` of its browser `browser`: a
    /// quit ends it, which its caller records, and a crash crashes it,
    /// counted once for its browser. Answers whether it quit.
    async fn session_lost(&self, open: &Session, browser: u64, lost: &Lost) -> bool {
        let (event, exit) = match lost {
            Lost::BrowserQuit => (SessionEvent::BrowserQuit, None),
            Lost::BrowserCrashed(exit) => (SessionEvent::BrowserCrash, exit.clone()),
            Lost::RendererCrashed | Lost::Destroyed | Lost::EngineClosed | Lost::FellBehind => {
                return false;
            }
        };
        match open.life.apply(event, exit) {
            Ok(SessionEffect::Quit) => true,
            Ok(SessionEffect::Crash) => {
                count_browser_crash(open, browser).await;
                false
            }
            Ok(_) => false,
            Err(error) => {
                tracing::error!(%error, "a Session's browser loss was refused");
                false
            }
        }
    }

    /// Follows the Session `open`, named `session`, as a task of its Run,
    /// until it ends or its Run stops: a panic of one of its owner's
    /// tasks, its seat's actor, its egress relay's or its pages' watches,
    /// closes it alone, with its pages, which raise the panic's
    /// `engine_panicked`, as its later calls do; its Run goes on (ADRs 0146,
    /// 0148). A page's own panic is its page owner's, which closes the page
    /// alone.
    pub(super) fn watch_session(&self, session: SessionId, open: &Arc<Session>) {
        let (sessions, open) = (self.clone(), Arc::clone(open));
        let (run, stopping) = (open.run.clone(), open.run.cancellation().clone());
        let span = identity(&run).span().clone();
        let task = async move {
            tokio::select! {
                () = open.owner.panicked() => {}
                () = open.life.ended() => return,
                () = stopping.cancelled() => return,
            }
            tracing::error!(
                session_id = %session.0,
                "a task of the Session's owner panicked: the engine closes the Session alone"
            );
            let error = session_panicked(
                open.run.metrics(),
                &session,
                open.owner.panicked_crash_report(),
            );
            // Recorded before the Session leaves the index, so a call that
            // no longer finds it raises the panic.
            open.life.panicked(error.clone());
            sessions.index.panicked(&session, &open, error.clone());
            let ended = match open.life.apply(SessionEvent::Close, None) {
                Ok(effect) => {
                    sessions
                        .end_session_with(&open, &session, effect, &Ending::Panicked(error))
                        .await
                }
                Err(error) => Err(error),
            };
            match ended {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "a Session a panic closed is not recorded closed");
                }
                Err(error) => tracing::error!(%error, "a Session's end after a panic was refused"),
            }
        };
        run.spawn(task.instrument(span));
    }

    /// Follows the losses of the browsers of `run`'s pool, `losses`, until
    /// the Run stops: every Session of the Run seated in a lost browser
    /// takes its loss, one that holds no page included (spec #594's S8 and
    /// S9).
    pub(super) fn follow_losses(&self, run: &Run, mut losses: mpsc::Receiver<BrowserLoss>) {
        let sessions = self.clone();
        let stopping = run.cancellation().clone();
        let run_id = run.id().clone();
        run.spawn(async move {
            loop {
                let loss = tokio::select! {
                    () = stopping.cancelled() => return,
                    loss = losses.recv() => match loss {
                        Some(loss) => loss,
                        None => return,
                    },
                };
                sessions.browser_lost(&run_id, &loss).await;
            }
        });
    }

    /// Every Session of the Run `run_id` seated where `loss` happened takes
    /// it.
    async fn browser_lost(&self, run_id: &RunId, loss: &BrowserLoss) {
        for (session, open) in self.index.sessions_of(run_id) {
            if !open.seat.was_at(&loss.at).await {
                continue;
            }
            if self
                .session_lost(&open, loss.at.browser(), &loss.lost)
                .await
            {
                if let Err(error) = self.session_ended(&open, &session).await {
                    tracing::warn!(%error, "a Session its browser's quit ended is not recorded closed");
                }
                stop_after_its_quit(open);
            }
        }
    }

    /// Follows the page `page_id`, which `page` is, of the Session `open`,
    /// as a task of the Session's owner, in its Run's browser `browser`,
    /// until it ends or its Session does: its lease ended, a
    /// panic of one of its owner's tasks (ADR 0148) or its reader fallen
    /// 4,096 events behind (ADR 0146), each of which closes its target, a
    /// target the browser destroyed, a renderer that
    /// crashed, or its browser lost, which ends its Session too when the
    /// Session is seated there. Once its Run stops, or the engine closes
    /// its browser, its calls in flight end with the Run's close (ADR
    /// 0113), with no wait for an end that never comes.
    pub(super) fn watch(&self, page_id: PageId, page: OpenPage, open: Arc<Session>, browser: u64) {
        let sessions = self.clone();
        let OpenPage {
            driver,
            owner,
            life,
            run,
            session,
            ..
        } = page;
        let (stopping, ended) = (run.cancellation().clone(), open.owner.child_token());
        let (open_owner, span) = (
            TaskStarter::clone(&open.owner),
            identity(&run).span().clone(),
        );
        let task = async move {
            let closed_by_the_engine = || {
                if let Err(error) = life.end(&Ending::RunStopped(run_stopped(&run))) {
                    tracing::error!(%error, "a page's end with its Run was refused");
                }
            };
            let lost = tokio::select! {
                lost = driver.lost() => lost,
                () = life.terminal() => return,
                () = stopping.cancelled() => return closed_by_the_engine(),
                // Its Session's end stops it, once that end ended the page,
                // unless its Run's stop is what ended the Session.
                () = ended.cancelled() => {
                    if stopping.is_cancelled() {
                        closed_by_the_engine();
                    }
                    return;
                }
                () = life.ended_by_lease() => {
                    // Its lease ended it (P31): the engine closes its target,
                    // whatever the lease, and the close is confirmed (P29).
                    driver.close().await;
                    match life.end(&Ending::Destroyed) {
                        Ok(effect) => sessions.page_ended(&page_id, &run, &life, effect).await,
                        Err(error) => tracing::error!(%error, "a page's end was refused"),
                    }
                    return;
                }
                panicked = owner.panicked() => {
                    tracing::error!(
                        page_id = %page_id.0,
                        "a task of the page's owner panicked: the engine closes the page alone"
                    );
                    let ending = (Ending::Panicked(panicked), ENGINE_PANICKED.name);
                    return sessions.close_alone(&page_id, &run, &life, &driver, ending).await;
                }
            };
            let ending = match &lost {
                Lost::RendererCrashed => Ending::RendererCrash,
                Lost::Destroyed => {
                    // The program's close confirmed, or one in the browser.
                    match life.destroyed() {
                        Ok(effect) => sessions.page_ended(&page_id, &run, &life, effect).await,
                        Err(error) => tracing::error!(%error, "a page's end was refused"),
                    }
                    return;
                }
                Lost::BrowserQuit => Ending::BrowserQuit,
                Lost::BrowserCrashed(exit) => Ending::BrowserCrash(exit.clone()),
                Lost::EngineClosed => return closed_by_the_engine(),
                Lost::FellBehind => {
                    tracing::warn!(
                        page_id = %page_id.0,
                        "the page's reader fell 4,096 events behind: the engine closes the page alone"
                    );
                    let ending = (Ending::EventsLost, PAGE_EVENTS_LOST.name);
                    return sessions
                        .close_alone(&page_id, &run, &life, &driver, ending)
                        .await;
                }
            };
            // The Session takes its browser's loss before the page's calls
            // answer, so a call the program makes next finds it ended.
            let quit = open.seat.is_in(browser).await
                && sessions.session_lost(&open, browser, &lost).await;
            match life.end(&ending) {
                Ok(effect) => {
                    // Its browser's crash is counted before its `crash`
                    // goes out, by whichever of the browser's Sessions and
                    // pages took it first.
                    if effect == PageEffect::BrowserCrash {
                        count_browser_crash(&open, browser).await;
                    }
                    sessions.page_ended(&page_id, &run, &life, effect).await;
                }
                Err(error) => tracing::error!(%error, "a page's end was refused"),
            }
            if quit {
                if let Err(error) = sessions.session_ended(&open, &session).await {
                    tracing::warn!(%error, "a Session its browser's quit ended is not recorded closed");
                }
                stop_after_its_quit(open);
            }
        };
        open_owner.spawn(task.instrument(span));
    }

    /// Closes the page `page_id` of `run`, whose state `life` is, through
    /// `driver`, as `ending` asks: a panic of one of its owner's tasks (ADR
    /// 0148), or its reader fallen behind (ADR 0146). Its calls in flight
    /// and its later calls raise what `ending` names, its Run trace records
    /// that code, `recorded`, and its target closes, the close confirmed,
    /// which ends every request it paused; its Session and its Run go on.
    async fn close_alone(
        &self,
        page_id: &PageId,
        run: &Run,
        life: &PageLife,
        driver: &PageDriver,
        (ending, recorded): (Ending, &str),
    ) {
        // The Run trace holds the Error code no reply may carry (ADR 0072).
        run.record_outcome(recorded);
        match life.end(&ending) {
            Ok(PageEffect::Close) => {}
            // Something else ended it first, which its own end acts on.
            Ok(_) => return,
            Err(error) => {
                tracing::error!(%error, "a page's end the engine makes was refused");
                return;
            }
        }
        driver.close().await;
        match life.end(&Ending::Destroyed) {
            Ok(effect) => self.page_ended(page_id, run, life, effect).await,
            Err(error) => tracing::error!(%error, "a page's end was refused"),
        }
    }
}

/// Stops the owner of the Session `open`, which its browser's quit ended,
/// as a task of its Run: the task that took the quit may be one of the
/// Session's own, which its stop waits for.
fn stop_after_its_quit(open: Arc<Session>) {
    let run = open.run.clone();
    run.spawn(async move { open.owner.stop().await });
}

/// Counts the crash of the browser `browser`, where a page of `open` was,
/// in its Run's Usage once for the browser (ADR 0040), or waits until the
/// first of the browser's Sessions and pages to take it counted it.
async fn count_browser_crash(open: &Session, browser: u64) {
    open.seat
        .crash_count_of(browser)
        .get_or_init(|| charge_a_crash(&open.run))
        .await;
}

/// Counts one crash in `run`'s Usage (ADR 0040).
async fn charge_a_crash(run: &Run) {
    if let Err(error) = run.charge(PAGE_CRASHES, 1).await {
        tracing::warn!(%error, "a crash was not counted in Usage");
    }
}
