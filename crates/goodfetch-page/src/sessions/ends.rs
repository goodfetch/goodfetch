//! How a page's end reaches its Session and its program (spec #594; ADRs
//! 0040 and 0113): each page is followed until it ends without the engine
//! closing its browser, and what its end did is acted on.

use std::sync::Arc;

use goodfetch_engine::runs::Run;
use goodfetch_protocol::page::events::{PAGE_CLOSE, PAGE_CRASH, PageEventName};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::usage::PAGE_CRASHES;

use super::life::Ending;
use super::lifecycle::{PageEffect, SessionEffect, SessionEvent};
use super::{OpenPage, Session, Sessions};
use crate::driver::Lost;

impl Sessions {
    /// Acts on what a page's end did (`effect`): a crash emits `crash` and
    /// counts in Usage (ADR 0040); a page that closed emits `close`, then
    /// leaves with its events and routes, so later calls raise `closed`.
    pub(super) async fn page_ended(
        &self,
        page_id: &PageId,
        session: &SessionId,
        run: &Run,
        effect: PageEffect,
    ) {
        let pages = std::slice::from_ref(page_id);
        match effect {
            PageEffect::RendererCrash | PageEffect::BrowserCrash => {
                if let Ok(events) = self.events.get(page_id) {
                    events.publish_end(PageEventName::Crash, &PAGE_CRASH).await;
                }
                if let Err(error) = run.charge(PAGE_CRASHES, 1).await {
                    tracing::warn!(%error, "a page's crash was not counted in Usage");
                }
            }
            PageEffect::ClosedInBrowser | PageEffect::Confirm | PageEffect::Quit => {
                if effect != PageEffect::Quit
                    && let Ok(events) = self.events.get(page_id)
                {
                    events.publish_end(PageEventName::Close, &PAGE_CLOSE).await;
                }
                self.state.lock().await.pages.remove(page_id);
                self.events.close(pages);
                self.interception.close(pages, session);
            }
            // A crashed page releases what is left, its admission and its
            // place in its browser, and stays `Crashed`: its later calls
            // raise `page_crashed` again (P33).
            PageEffect::Release => {
                let mut state = self.state.lock().await;
                if let Some(page) = state.pages.remove(page_id) {
                    state
                        .crashed
                        .insert(page_id.clone(), (run.id().clone(), page.life));
                }
                drop(state);
                self.events.close(pages);
                self.interception.close(pages, session);
            }
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

    /// Follows the page `page_id`, which `page` is, of the Session `open`,
    /// in its Run's browser `browser`, until it ends: a target the browser
    /// destroyed, a renderer that crashed, or its browser lost, which ends
    /// its Session too when the Session is seated there.
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
            let lost = tokio::select! {
                lost = driver.lost() => lost,
                () = life.terminal() => return,
                () = stopping.cancelled() => return,
            };
            let (ending, session_event) = match lost {
                Lost::RendererCrashed => (Ending::RendererCrash, None),
                Lost::Destroyed => (Ending::Destroyed, None),
                Lost::BrowserQuit => (
                    Ending::BrowserQuit,
                    Some((SessionEvent::BrowserQuit, None)),
                ),
                Lost::BrowserCrashed(exit) => (
                    Ending::BrowserCrash(exit.clone()),
                    Some((SessionEvent::BrowserCrash, exit)),
                ),
                Lost::EngineClosed => return,
            };
            match life.end(&ending) {
                Ok(effect) => sessions.page_ended(&page_id, &session, &run, effect).await,
                Err(error) => tracing::error!(%error, "a page's end was refused"),
            }
            let Some((event, exit)) = session_event else {
                return;
            };
            if !open.seat.is_in(browser).await {
                return;
            }
            match open.life.apply(event, exit) {
                Ok(SessionEffect::Quit) => {
                    if let Err(error) = sessions.session_ended(&open, &session).await {
                        tracing::warn!(%error, "a Session its browser's quit ended is not recorded closed");
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::error!(%error, "a Session's browser loss was refused"),
            }
        });
    }
}
