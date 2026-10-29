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
//! 2026-10-28).

use std::sync::Arc;

use futures_util::FutureExt as _;
use goodfetch_engine::CallError;
use goodfetch_engine::run::limit_reached;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::connection::{ClosedValue, StoppedBy};
use goodfetch_protocol::page::events::{PAGE_CLOSE, PAGE_CRASH, PageEventName};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::run::StopReason;
use goodfetch_protocol::usage::PAGE_CRASHES;
use tokio::sync::mpsc;

use super::life::Ending;
use super::lifecycle::{PageEffect, SessionEffect, SessionEvent};
use super::{OpenPage, Session, Sessions};
use crate::driver::Lost;
use crate::errors::cancelled;
use crate::pool::BrowserLoss;

impl Sessions {
    /// Acts on what a page's end did (`effect`): a crash emits `crash`, a
    /// renderer's counting in Usage (ADR 0040); a page that closed emits
    /// `close`, then leaves with its events and routes, so later calls raise
    /// `closed`; a page whose browser crashed leaves its browser, as a
    /// released one does.
    pub(super) async fn page_ended(&self, page_id: &PageId, run: &Run, effect: PageEffect) {
        let pages = std::slice::from_ref(page_id);
        match effect {
            PageEffect::RendererCrash | PageEffect::BrowserCrash => {
                if let Ok(events) = self.events.get(page_id) {
                    events.publish_end(PageEventName::Crash, &PAGE_CRASH).await;
                }
                if effect == PageEffect::RendererCrash {
                    charge_a_crash(run).await;
                } else {
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
                    && let Ok(events) = self.events.get(page_id)
                {
                    events.publish_end(PageEventName::Close, &PAGE_CLOSE).await;
                }
                self.state.lock().await.pages.remove(page_id);
                self.events.close(pages);
                self.dialogs.close(pages);
                self.interception.close_pages(pages);
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
        if let Some(page) = state.pages.remove(page_id) {
            state
                .crashed
                .insert(page_id.clone(), (run.id().clone(), page.life));
        }
        drop(state);
        self.events.close(pages);
        self.dialogs.close(pages);
        self.interception.close_pages(pages);
    }

    /// The Session `open` takes the loss `lost` of its browser `browser`: a
    /// quit ends it, which its caller records, and a crash crashes it,
    /// counted once for its browser. Answers whether it quit.
    async fn session_lost(&self, open: &Session, browser: u64, lost: &Lost) -> bool {
        let (event, exit) = match lost {
            Lost::BrowserQuit => (SessionEvent::BrowserQuit, None),
            Lost::BrowserCrashed(exit) => (SessionEvent::BrowserCrash, exit.clone()),
            Lost::RendererCrashed | Lost::Destroyed | Lost::EngineClosed => return false,
        };
        match open.life.apply(event, exit) {
            Ok(SessionEffect::Quit) => true,
            Ok(SessionEffect::Crash) => {
                if open.seat.first_crash_of(browser) {
                    charge_a_crash(&open.run).await;
                }
                false
            }
            Ok(_) => false,
            Err(error) => {
                tracing::error!(%error, "a Session's browser loss was refused");
                false
            }
        }
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
        let seated: Vec<(SessionId, Arc<Session>)> = self
            .state
            .lock()
            .await
            .sessions
            .iter()
            .filter(|(_, open)| open.run.id() == run_id)
            .map(|(session, open)| (session.clone(), Arc::clone(open)))
            .collect();
        for (session, open) in seated {
            if !open.seat.was_at(&loss.at).await {
                continue;
            }
            if self
                .session_lost(&open, loss.at.browser(), &loss.lost)
                .await
                && let Err(error) = self.session_ended(&open, &session).await
            {
                tracing::warn!(%error, "a Session its browser's quit ended is not recorded closed");
            }
        }
    }

    /// Follows the page `page_id`, which `page` is, of the Session `open`,
    /// in its Run's browser `browser`, until it ends: its lease ended, which
    /// closes its target, a target the browser destroyed, a renderer that
    /// crashed, or its browser lost, which ends its Session too when the
    /// Session is seated there. Once its Run stops, or the engine closes
    /// its browser, its calls in flight end as its Session's close by the
    /// program does (S7), with no wait for an end that never comes.
    pub(super) fn watch(&self, page_id: PageId, page: OpenPage, open: Arc<Session>, browser: u64) {
        let sessions = self.clone();
        let OpenPage {
            driver,
            life,
            run,
            session,
            ..
        } = page;
        let stopping = run.cancellation().clone();
        run.clone().spawn(async move {
            let closed_by_the_engine = || {
                if let Err(error) = life.end(&Ending::RunStopped(run_stop(&run))) {
                    tracing::error!(%error, "a page's end with its Run was refused");
                }
            };
            let lost = tokio::select! {
                lost = driver.lost() => lost,
                () = life.terminal() => return,
                () = stopping.cancelled() => return closed_by_the_engine(),
                () = life.ended_by_lease() => {
                    // Its lease ended it (P31): the engine closes its target,
                    // whatever the lease, and the close is confirmed (P29).
                    driver.close().await;
                    match life.end(&Ending::Destroyed) {
                        Ok(effect) => sessions.page_ended(&page_id, &run, effect).await,
                        Err(error) => tracing::error!(%error, "a page's end was refused"),
                    }
                    return;
                }
            };
            let ending = match &lost {
                Lost::RendererCrashed => Ending::RendererCrash,
                Lost::Destroyed => {
                    // The program's close confirmed, or one in the browser.
                    match life.destroyed() {
                        Ok(effect) => sessions.page_ended(&page_id, &run, effect).await,
                        Err(error) => tracing::error!(%error, "a page's end was refused"),
                    }
                    return;
                }
                Lost::BrowserQuit => Ending::BrowserQuit,
                Lost::BrowserCrashed(exit) => Ending::BrowserCrash(exit.clone()),
                Lost::EngineClosed => return closed_by_the_engine(),
            };
            // The Session takes its browser's loss before the page's calls
            // answer, so a call the program makes next finds it ended.
            let quit = open.seat.is_in(browser).await
                && sessions.session_lost(&open, browser, &lost).await;
            match life.end(&ending) {
                Ok(effect) => sessions.page_ended(&page_id, &run, effect).await,
                Err(error) => tracing::error!(%error, "a page's end was refused"),
            }
            if quit && let Err(error) = sessions.session_ended(&open, &session).await {
                tracing::warn!(%error, "a Session its browser's quit ended is not recorded closed");
            }
        });
    }
}

/// What a call in flight raises when `run` stops under it: the Limit the
/// Run reached, as the charge that reached it raised (ADR 0041), and
/// otherwise the program's close of its Session (S7), whatever stopped it.
fn run_stop(run: &Run) -> CallError {
    match run.stopping().now_or_never() {
        Some(StopReason::Limit(reached)) => limit_reached(&reached),
        _ => cancelled(StoppedBy::Program, ClosedValue::Session),
    }
}

/// Counts one crash in `run`'s Usage (ADR 0040).
async fn charge_a_crash(run: &Run) {
    if let Err(error) = run.charge(PAGE_CRASHES, 1).await {
        tracing::warn!(%error, "a crash was not counted in Usage");
    }
}
