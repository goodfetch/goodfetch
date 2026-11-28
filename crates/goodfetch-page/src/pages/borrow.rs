//! Borrowing the program's own Playwright or Puppeteer browser over the SDK's
//! JSON-RPC connection (`run.borrow()`, ADRs 0056, 0078, 0089 and 0140;
//! ticket #744).
//!
//! Each `run.borrow` call is a bidirectional credit-controlled relay:
//! - Engine -> client commands flow as `$/borrow_command` notifications gated
//!   by the client's `$/credit` grants (`CallStream::spend`).
//! - Client -> engine CDP responses and events flow as `$/borrow_frame` input
//!   notifications gated by the engine's `$/credit` grants (`CallInput`).
//! - Frames cross as raw UTF-8 JSON text and are fed directly into
//!   `goodfetch-cdp::Connection::new` over a duplex pipe without parsing into
//!   a `Value` twice.
//! - Each borrowed browser context maps to one Session of the Run; closing
//!   the Run or cancelling the borrow never closes the borrowed browser.

use std::sync::Arc;

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::page::borrow::{
    BORROW_COMMAND, BORROW_FRAME, BORROW_READY, BORROW_REQUIRES_CDP, BorrowCommandToClient,
    BorrowEnded, BorrowFrameFromClient, BorrowReady, BorrowRequiresCdpDetails, RUN_BORROW,
    RunBorrowParams,
};
use goodfetch_protocol::page::session::LOGIN_BORROWED;
use serde_json::json;

use super::Pages;
use crate::approvals::{ApprovalAction, ApprovalWait, caller_actor_label, record_login_audit};
use crate::cdp::borrowed::connect_relayed;
use crate::cdp::driver::BorrowedBrowser;
use crate::trace;

/// How many unread `$/borrow_frame` notifications the client may have in
/// flight to the engine under the engine's `$/credit` grants.
const BORROW_INPUT_ROOM: usize = 64;

/// Registers the `run.borrow` handler.
pub(super) fn with_borrow_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let borrowing = pages.clone();
    contribution.with_handler(
        Handler::new(
            &RUN_BORROW,
            move |params: RunBorrowParams, context: CallContext| {
                let pages = borrowing.clone();
                async move { pages.borrow(params, context).await }
            },
        )
        .streaming()
        .taking_input(BORROW_FRAME, BORROW_INPUT_ROOM),
    )
}

impl Pages {
    /// Relays a Chromium-family browser over the SDK's JSON-RPC connection for
    /// `params.run_id` under credits until `$/cancel`, browser retirement, or
    /// Run end (ADRs 0056, 0078, 0089).
    async fn borrow(
        &self,
        params: RunBorrowParams,
        context: CallContext,
    ) -> Result<BorrowEnded, CallError> {
        check_borrow_browser(params.browser.as_deref())?;

        let run = self.sessions.live_run(&params.run_id, &context).await?;
        self.approve_borrow(&run, &context).await?;
        let browser_owner = run.identity.owner().child();
        let on = run.identity.starting_on(&browser_owner);
        let (connection, outgoing, incoming) =
            connect_relayed((&run, &on), "client_browser").await?;

        let call_id = context.id().clone();
        // Child of `browser_owner` so Run end immediately cancels both relay
        // pumps and closes the duplex pipe without waiting on `borrow()`.
        let relay_ended = browser_owner.child_token();
        spawn_relay_pumps(&on, &context, &call_id, &relay_ended, outgoing, incoming);

        let borrowed_browser = Arc::new(BorrowedBrowser::relayed(
            Arc::clone(&run),
            browser_owner,
            connection,
            "client_browser".to_owned(),
        ));

        let run_ended = run.identity.owner().child_token();
        let setup_result = tokio::select! {
            res = self.setup_borrow(&params, &borrowed_browser, &context) => Some(res),
            () = context.cancellation().cancelled() => None,
            () = relay_ended.cancelled() => None,
            () = run_ended.cancelled() => None,
        };

        let Some(setup_result) = setup_result else {
            let reason = end_reason(&run_ended, context.cancellation());
            relay_ended.cancel();
            borrowed_browser.stop().await;
            return Ok(BorrowEnded {
                reason: reason.to_owned(),
            });
        };

        let (session_id, page_id, browser_context_id) = match setup_result {
            Ok(ready) => ready,
            Err(error) => {
                relay_ended.cancel();
                borrowed_browser.stop().await;
                return Err(error);
            }
        };

        record_borrow_audit(&context, &run, &session_id, page_id.as_ref()).await;
        trace::provenance(
            &run.trace,
            json!({
                "action": "borrowed_browser_attached",
                "unguarded": true,
                "session_id": session_id,
                "page_id": page_id,
                "browser_context_id": browser_context_id,
            }),
        )
        .await;

        if let Some(pid) = &page_id {
            self.watch_borrowed_page(pid).await;
        }

        let ready_sent = context
            .stream()
            .notify(
                BORROW_READY,
                &BorrowReady {
                    id: call_id,
                    session_id,
                    page_id: page_id.clone(),
                    browser_context_id,
                },
            )
            .await;

        let reason = if ready_sent {
            tokio::select! {
                () = context.cancellation().cancelled() => end_reason(&run_ended, context.cancellation()),
                () = relay_ended.cancelled() => if run_ended.is_cancelled() { "run_ended" } else { "browser_closed" },
                () = run_ended.cancelled() => "run_ended",
            }
        } else {
            "browser_closed"
        };

        relay_ended.cancel();
        if reason != "run_ended"
            && let Some(pid) = &page_id
        {
            self.sessions.mark_borrowed_page_closed(pid).await;
        }
        borrowed_browser.stop().await;
        Ok(BorrowEnded {
            reason: reason.to_owned(),
        })
    }

    async fn approve_borrow(
        &self,
        run: &crate::browser_run::BrowserRun,
        context: &CallContext,
    ) -> Result<(), CallError> {
        let is_recipe_run = run.identity.is_recipe_run();
        if !(context.surface().acts_for_model() || is_recipe_run) {
            return Ok(());
        }
        self.approvals
            .wait_for_approval(ApprovalWait {
                action: ApprovalAction::BorrowLogin {
                    source_kind: "client_browser",
                    domains: Vec::new(),
                    target: format!("Run `{}`", run.identity.id().as_str()),
                },
                actor: caller_actor_label(context, is_recipe_run),
                run: Some(run),
                engine: context.engine(),
                cancellation: context.cancellation().clone(),
            })
            .await?;
        Ok(())
    }

    /// Resolves the borrowed browser context, opens or joins its Session, and
    /// attaches `params.target_id` when borrowing a specific page.
    async fn setup_borrow(
        &self,
        params: &RunBorrowParams,
        borrowed_browser: &Arc<BorrowedBrowser>,
        context: &CallContext,
    ) -> Result<
        (
            goodfetch_protocol::page::SessionId,
            Option<goodfetch_protocol::page::PageId>,
            String,
        ),
        CallError,
    > {
        let (browser_context_id, is_default_context) = match &params.browser_context_id {
            Some(ctx) if !ctx.is_empty() => {
                let (default_ctx, _) = borrowed_browser.resolve_context(None).await?;
                let is_default = ctx == &default_ctx;
                (ctx.clone(), is_default)
            }
            _ => {
                borrowed_browser
                    .resolve_context(params.target_id.as_deref())
                    .await?
            }
        };
        let session_id = self
            .sessions
            .open_relayed_session(
                &params.run_id,
                Arc::clone(borrowed_browser),
                browser_context_id.clone(),
                is_default_context,
                context,
            )
            .await?;
        let page_id = match &params.target_id {
            Some(target_id) => Some(
                self.sessions
                    .attach_relayed_page(&session_id, target_id.clone(), context)
                    .await?,
            ),
            None => None,
        };
        Ok((session_id, page_id, browser_context_id))
    }

    /// Watches a borrowed page's CDP events and marks the Collie Page closed
    /// when the crawler closes the tab or the relay ends; exits at once if the
    /// page's own owner is already stopping.
    async fn watch_borrowed_page(&self, page_id: &goodfetch_protocol::page::PageId) {
        let Ok(open_page) = self.sessions.page(page_id).await else {
            return;
        };
        let watch_sessions = self.sessions.clone();
        let watch_pid = page_id.clone();
        let page_stopping = open_page.owner.tasks().owner().child_token();
        let mut events = open_page.driver.events();
        open_page.owner.tasks().spawn(async move {
            loop {
                tokio::select! {
                    () = page_stopping.cancelled() => return,
                    ev = events.next() => {
                        if ev.is_none() {
                            break;
                        }
                    }
                }
            }
            if !page_stopping.is_cancelled() {
                watch_sessions.mark_borrowed_page_closed(&watch_pid).await;
            }
        });
    }
}

/// Classifies why a `run.borrow()` call ended before or after setup.
fn end_reason(
    run_ended: &tokio_util::sync::CancellationToken,
    cancelled: &tokio_util::sync::CancellationToken,
) -> &'static str {
    if run_ended.is_cancelled() {
        "run_ended"
    } else if cancelled.is_cancelled() {
        "cancelled"
    } else {
        "browser_closed"
    }
}

/// Spawns the engine -> client (`$/borrow_command`) and client -> engine
/// (`$/borrow_frame`) relay pumps under credits on `on`.
fn spawn_relay_pumps(
    on: &goodfetch_engine::run_identity::RunIdentity,
    context: &CallContext,
    call_id: &goodfetch_protocol::jsonrpc::RequestId,
    relay_ended: &tokio_util::sync::CancellationToken,
    mut outgoing: crate::cdp::borrowed::RelayedOutgoing,
    mut incoming: crate::cdp::borrowed::RelayedIncoming,
) {
    let out_context = context.clone();
    let out_id = call_id.clone();
    let out_ended = relay_ended.clone();
    on.spawn(async move {
        let stream = out_context.stream();
        loop {
            let frame = tokio::select! {
                () = out_ended.cancelled() => break,
                frame = outgoing.next_frame() => match frame {
                    Some(frame) => frame,
                    None => break,
                },
            };
            let granted = tokio::select! {
                () = out_ended.cancelled() => break,
                granted = stream.spend(1) => granted,
            };
            if granted == 0 {
                break;
            }
            let sent = stream
                .notify(
                    BORROW_COMMAND,
                    &BorrowCommandToClient {
                        id: out_id.clone(),
                        frame,
                    },
                )
                .await;
            if !sent {
                break;
            }
        }
        out_ended.cancel();
    });

    let in_context = context.clone();
    let in_id = call_id.clone();
    let in_ended = relay_ended.clone();
    on.spawn(async move {
        let input = in_context.input();
        let initial = u32::try_from(BORROW_INPUT_ROOM).unwrap_or(64);
        if !input
            .notify(
                CREDIT,
                &CreditParams {
                    id: in_id.clone(),
                    credits: initial,
                },
            )
            .await
        {
            in_ended.cancel();
            return;
        }
        loop {
            let received = tokio::select! {
                () = in_ended.cancelled() => break,
                next = input.next() => match next {
                    Ok(Some(received)) => received,
                    Ok(None) | Err(_) => break,
                },
            };
            let Ok(msg) = received.read::<BorrowFrameFromClient>() else {
                break;
            };
            if msg.end {
                break;
            }
            if let Some(frame) = msg.frame {
                let wrote = tokio::select! {
                    () = in_ended.cancelled() => break,
                    wrote = incoming.write_frame(&frame) => wrote,
                };
                if wrote.is_err() {
                    break;
                }
            }
            if !input
                .notify(
                    CREDIT,
                    &CreditParams {
                        id: in_id.clone(),
                        credits: 1,
                    },
                )
                .await
            {
                break;
            }
        }
        drop(incoming);
        in_ended.cancel();
    });
}

/// Refuses Playwright's Firefox and WebKit (and any non-Chromium engine) with
/// `borrow_requires_cdp` (`ProgramError`, ADRs 0056 and 0089).
fn check_borrow_browser(browser: Option<&str>) -> Result<(), CallError> {
    let Some(browser) = browser else {
        return Ok(());
    };
    let lower = browser.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "" | "chromium" | "chrome" | "edge" | "brave" | "msedge"
    ) {
        return Ok(());
    }
    Err(CallError::new(
        &BORROW_REQUIRES_CDP,
        format!(
            "`run.borrow()` requires a Chromium-family browser speaking CDP; `{browser}` exposes no CDP (ADRs 0056 and 0089)."
        ),
        &BorrowRequiresCdpDetails {
            browser: browser.to_owned(),
            fix: "Borrow a Chromium-family browser (Chrome, Edge, Brave, or Playwright Chromium), or use `cdp_url`.".to_owned(),
        },
    ))
}

async fn record_borrow_audit(
    context: &CallContext,
    run: &crate::browser_run::BrowserRun,
    session_id: &goodfetch_protocol::page::SessionId,
    page_id: Option<&goodfetch_protocol::page::PageId>,
) {
    let details = json!({
        "session_id": session_id,
        "page_id": page_id,
        "source_kind": "client_browser",
    });
    record_login_audit(
        context,
        Some(run.identity.id().clone()),
        None,
        &LOGIN_BORROWED,
        details,
    )
    .await;
}
