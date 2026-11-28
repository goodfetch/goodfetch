//! Raw CDP sessions (`page.newCDPSession()`, ADRs 0040, 0085, 0087, 0092 and
//! 0140): relayed over the program's own connection, reaching only its page's
//! target, refusing target-attaching/closing and browser-closing commands with
//! `cdp_command_refused`, marking results unguarded, recording the attach in
//! the Audit log, stopping Secret typing while open, holding the page for
//! area 3's pool, and binding frames to the page's lease check.

use std::collections::BTreeMap;
#[expect(
    clippy::disallowed_types,
    reason = "short-lived CDP session table lock outside loom tests"
)]
use std::sync::Mutex;
use std::sync::{Arc, PoisonError};

use goodfetch_engine::audit::Origin;
use goodfetch_engine::events::{Envelope, EventData};
use goodfetch_engine::governor::PageHold;
use goodfetch_engine::rpc::send_events;
use goodfetch_engine::{CallContext, CallError, CallStream, Contribution, Handler};
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::events::Position;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::cdp_session::{
    CDP_COMMAND_REFUSED, CDP_SESSION_ATTACHED, CDP_SESSION_CLOSE, CDP_SESSION_EVENT,
    CDP_SESSION_ON, CDP_SESSION_SEND, CdpCommandRefusedDetails, CdpSessionAnswer,
    CdpSessionCloseParams, CdpSessionClosed, CdpSessionEvent, CdpSessionId, CdpSessionOnEnded,
    CdpSessionOnParams, CdpSessionOpened, CdpSessionSendParams, PAGE_NEW_CDP_SESSION,
    PageNewCdpSessionParams,
};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::{Pages, in_call};
use crate::access_mode::BrowserFamily;
use crate::cdp::driver::OtherCodeGuard;
use crate::errors::Call;
use crate::pool::Hold;
use crate::sessions::OpenPage;
use crate::trace;

/// The Provenance action recorded when a raw CDP session attaches.
const CDP_SESSION_ATTACHED_ACTION: &str = "cdp_session_attached";

/// The most raw CDP events sent in one `$/events` batch.
const BATCH_LIMIT: usize = 64;

/// Active raw CDP sessions across all open pages.
#[derive(Clone, Default)]
pub(super) struct CdpSessions {
    #[expect(
        clippy::disallowed_types,
        reason = "short-lived CDP session table lock outside loom tests"
    )]
    inner: Arc<Mutex<CdpSessionsState>>,
}

#[derive(Default)]
struct CdpSessionsState {
    next_id: u64,
    sessions: BTreeMap<CdpSessionId, Arc<OpenCdpSession>>,
}

struct OpenCdpSession {
    id: CdpSessionId,
    page_id: PageId,
    page: OpenPage,
    /// Marks the page as readable by other code so Secret placeholders raise
    /// `secret_input_refused` while this CDP session is open (ADR 0092).
    _other_code: OtherCodeGuard,
    /// Holds the page's browser for area 3's pool while this CDP session is
    /// open (ADR 0040).
    _pool_hold: Hold,
    /// Signaled once this CDP session closes.
    closed: watch::Sender<bool>,
}

impl CdpSessions {
    fn insert(&self, page_id: PageId, page: OpenPage) -> (CdpSessionId, Arc<OpenCdpSession>) {
        let other_code = page.driver.attach_other_code();
        let pool_hold = page.hold(PageHold::Held);
        let (closed, _) = watch::channel(false);
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        state.next_id += 1;
        let id = CdpSessionId(format!("{}.cdp-{}", page_id.0, state.next_id));
        let open = Arc::new(OpenCdpSession {
            id: id.clone(),
            page_id,
            page,
            _other_code: other_code,
            _pool_hold: pool_hold,
            closed,
        });
        state.sessions.insert(id.clone(), Arc::clone(&open));
        (id, open)
    }

    fn get(&self, id: &CdpSessionId) -> Option<Arc<OpenCdpSession>> {
        let state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        state.sessions.get(id).cloned()
    }

    fn remove(&self, id: &CdpSessionId) -> Option<Arc<OpenCdpSession>> {
        let removed = {
            let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            state.sessions.remove(id)
        };
        if let Some(open) = &removed {
            open.closed.send_replace(true);
        }
        removed
    }
}

/// Adds the handlers of `page.new_cdp_session`, `cdp_session.send`,
/// `cdp_session.close` and `cdp_session.on`.
pub(super) fn with_cdp_session_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (opening, sending, closing, listening) =
        (pages.clone(), pages.clone(), pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_NEW_CDP_SESSION,
            move |params: PageNewCdpSessionParams, context: CallContext| {
                let pages = opening.clone();
                in_call((PAGE_NEW_CDP_SESSION.name, context.form()), async move {
                    pages.new_cdp_session(params).await
                })
            },
        ))
        .with_handler(Handler::new(
            &CDP_SESSION_SEND,
            move |params: CdpSessionSendParams, context: CallContext| {
                let pages = sending.clone();
                in_call((CDP_SESSION_SEND.name, context.form()), async move {
                    pages.cdp_session_send(params).await
                })
            },
        ))
        .with_handler(Handler::new(
            &CDP_SESSION_CLOSE,
            move |params: CdpSessionCloseParams, context: CallContext| {
                let pages = closing.clone();
                in_call((CDP_SESSION_CLOSE.name, context.form()), async move {
                    Ok(pages.cdp_session_close(&params))
                })
            },
        ))
        .with_handler(
            Handler::new(
                &CDP_SESSION_ON,
                move |params: CdpSessionOnParams, context: CallContext| {
                    let pages = listening.clone();
                    async move { pages.cdp_session_on(params, context.stream()).await }
                },
            )
            .streaming(),
        )
}

impl Pages {
    /// Opens a raw CDP session on `params.page_id` (ADR 0085, ticket #751).
    async fn new_cdp_session(
        &self,
        params: PageNewCdpSessionParams,
    ) -> Result<CdpSessionOpened, CallError> {
        let page = self.sessions.page(&params.page_id).await?;
        check_chromium_family(&page)?;
        check_lease(&page, &params.page_id)?;

        let (cdp_session_id, open) = self
            .cdp_sessions
            .insert(params.page_id.clone(), page.clone());

        // Record the attach in the Audit log and mark results unguarded in Provenance.
        record_attach_audit(&page, &params.page_id, &cdp_session_id).await;
        trace::provenance(
            &page.run.trace,
            json!({
                "action": CDP_SESSION_ATTACHED_ACTION,
                "unguarded": true,
                "page_id": params.page_id,
                "session_id": page.session,
                "cdp_session_id": cdp_session_id,
            }),
        )
        .await;

        // Release the CDP session automatically if its page detaches.
        let sessions = self.cdp_sessions.clone();
        let watch_id = cdp_session_id.clone();
        let mut events = open.page.driver.events();
        let mut closed = open.closed.subscribe();
        open.page.owner.tasks().spawn(async move {
            loop {
                tokio::select! {
                    changed = closed.changed() => {
                        if changed.is_err() || *closed.borrow() {
                            break;
                        }
                    }
                    next = events.next() => {
                        if next.is_none() {
                            sessions.remove(&watch_id);
                            break;
                        }
                    }
                }
            }
        });

        Ok(CdpSessionOpened { cdp_session_id })
    }

    /// Sends a raw CDP command on an open CDP session without queuing behind a
    /// Collie call (ADR 0085, ticket #751).
    async fn cdp_session_send(
        &self,
        params: CdpSessionSendParams,
    ) -> Result<CdpSessionAnswer, CallError> {
        let open = self
            .cdp_sessions
            .get(&params.cdp_session_id)
            .ok_or_else(|| cdp_session_closed(&params.cdp_session_id))?;

        if let Some(reason) = refused_command(&params.method, open.page.driver.is_borrowed()) {
            return Err(CallError::new(
                &CDP_COMMAND_REFUSED,
                format!("CDP command `{}` refused: {reason}", params.method),
                &CdpCommandRefusedDetails {
                    method: params.method,
                    reason: reason.to_owned(),
                },
            ));
        }

        if check_lease(&open.page, &open.page_id).is_err() {
            self.cdp_sessions.remove(&open.id);
            return Err(attach_ended_error(&open.id));
        }

        let arguments = params.params.unwrap_or_else(|| json!({}));
        let call = Call::Action {
            operation: CDP_SESSION_SEND.name,
        };
        let result = open
            .page
            .driver
            .send_raw_cdp(params.method, arguments, &call)
            .await
            .map_err(|error| {
                if check_lease(&open.page, &open.page_id).is_err() {
                    self.cdp_sessions.remove(&open.id);
                    attach_ended_error(&open.id)
                } else {
                    error
                }
            })?;
        Ok(CdpSessionAnswer { result })
    }

    /// Closes an open CDP session, releasing its external-reader mark and
    /// pool hold; closing an already-closed session succeeds.
    fn cdp_session_close(&self, params: &CdpSessionCloseParams) -> CdpSessionClosed {
        self.cdp_sessions.remove(&params.cdp_session_id);
        CdpSessionClosed {}
    }

    /// Streams raw CDP events matching `params.event` on `params.cdp_session_id`
    /// until cancelled or the session, page, or lease ends.
    async fn cdp_session_on(
        &self,
        params: CdpSessionOnParams,
        stream: &CallStream,
    ) -> Result<CdpSessionOnEnded, CallError> {
        let open = self
            .cdp_sessions
            .get(&params.cdp_session_id)
            .ok_or_else(|| cdp_session_closed(&params.cdp_session_id))?;
        if check_lease(&open.page, &open.page_id).is_err() {
            self.cdp_sessions.remove(&open.id);
            return Err(attach_ended_error(&open.id));
        }

        let mut subscription = open.page.driver.events();
        let mut closed = open.closed.subscribe();
        let mut position = 0u64;

        loop {
            let event = tokio::select! {
                biased;
                changed = closed.changed() => {
                    if changed.is_err() || *closed.borrow() {
                        break;
                    }
                    continue;
                }
                next = subscription.next() => {
                    let Some(event) = next else {
                        break;
                    };
                    event
                }
            };
            if event.method != params.event {
                continue;
            }
            if check_lease(&open.page, &open.page_id).is_err() {
                self.cdp_sessions.remove(&open.id);
                return Err(attach_ended_error(&open.id));
            }
            let payload: Value = event.raw_params();
            let credits = stream.spend(BATCH_LIMIT).await;
            position += 1;
            let data = EventData::encode(
                &CdpSessionEvent {
                    cdp_session_id: open.id.clone(),
                    method: event.method.clone(),
                    params: payload,
                },
                open.page.driver.work(),
            );
            stream.refund(credits - 1);
            let batch = vec![Envelope {
                position: Position(position),
                event: CDP_SESSION_EVENT.name,
                data,
                dropped: 0,
            }];
            if !send_events(stream, batch).await {
                break;
            }
        }

        Ok(CdpSessionOnEnded {})
    }
}

/// Refuses raw CDP commands that attach or close other targets, close the
/// page target, or close/crash the browser (ADRs 0085 and 0089).
pub(crate) fn refused_command(method: &str, borrowed: bool) -> Option<&'static str> {
    match method {
        "Target.attachToTarget"
        | "Target.attachToBrowserTarget"
        | "Target.setAutoAttach"
        | "Target.exposeDevToolsProtocol" => {
            Some("a page CDP session cannot attach to other targets")
        }
        "Target.closeTarget" | "Target.createTarget" => {
            Some("a page CDP session cannot create or close targets")
        }
        "Target.createBrowserContext" | "Target.disposeBrowserContext" => {
            Some("a page CDP session cannot create or dispose browser contexts")
        }
        "Page.close" => Some("a page CDP session cannot close its target; use `page.close`"),
        "Browser.close" | "Browser.crash" | "Browser.crashGpuProcess" => {
            if borrowed {
                Some("a Borrowed browser cannot be closed or crashed from a CDP session")
            } else {
                Some("a page CDP session cannot close or crash the browser")
            }
        }
        _ => None,
    }
}

fn check_chromium_family(page: &OpenPage) -> Result<(), CallError> {
    let product_non_chromium = page.driver.browser_product().is_some_and(|product| {
        let lower = product.to_ascii_lowercase();
        lower.contains("firefox") || lower.contains("webkit") || lower.contains("safari")
    });
    if page.browser_family != BrowserFamily::Chromium || product_non_chromium {
        let kind = match page.browser_family {
            BrowserFamily::Firefox => "firefox",
            BrowserFamily::WebKit => "webkit",
            BrowserFamily::Chromium => "non_chromium",
        };
        return Err(CallError::new(
            &CAPABILITY_UNAVAILABLE,
            "raw CDP sessions (`page.newCDPSession()`) are only available on Chromium, not Firefox or WebKit (ADR 0085)",
            &CapabilityUnavailableDetails {
                key: PAGE_NEW_CDP_SESSION.name.to_owned(),
                layer: SettingsLayer::Run,
                kind: kind.to_owned(),
                fix: "use a Chromium browser for raw CDP sessions".to_owned(),
            },
        ));
    }
    Ok(())
}

fn check_lease(page: &OpenPage, page_id: &PageId) -> Result<(), CallError> {
    if let Some(lease) = page.lease()
        && lease.check().is_err()
    {
        return Err(CallError::new(
            &CLOSED,
            format!(
                "the work item's lease for page `{}` ended (`attach_ended`)",
                page_id.0
            ),
            &json!({ "page_id": page_id.0, "reason": "attach_ended" }),
        ));
    }
    Ok(())
}

fn cdp_session_closed(id: &CdpSessionId) -> CallError {
    CallError::new(
        &CLOSED,
        format!("CDP session `{}` is closed", id.0),
        &json!({ "cdp_session_id": id.0 }),
    )
}

fn attach_ended_error(id: &CdpSessionId) -> CallError {
    CallError::new(
        &CLOSED,
        format!(
            "CDP session `{}` ended because its work item's lease expired (`attach_ended`)",
            id.0
        ),
        &json!({ "cdp_session_id": id.0, "reason": "attach_ended" }),
    )
}

async fn record_attach_audit(page: &OpenPage, page_id: &PageId, cdp_session_id: &CdpSessionId) {
    let origin = Origin {
        actor: Actor::Engine,
        surface: None,
        layer: None,
        run_id: Some(page.run.identity.id().clone()),
    };
    let now =
        Timestamp::try_from(page.run.identity.point_in_time()).unwrap_or(Timestamp::UNIX_EPOCH);
    let details = json!({
        "page_id": page_id,
        "session_id": page.session,
        "cdp_session_id": cdp_session_id,
    });
    if let Err(error) = page
        .run
        .engine
        .audit()
        .record(&origin, &CDP_SESSION_ATTACHED, details, now)
        .await
    {
        tracing::warn!(%error, page_id = %page_id.0, "could not record CDP session attach in Audit log");
    }
}

#[cfg(test)]
mod tests;
