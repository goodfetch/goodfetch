//! Page lending handlers (`page.lend`, `cdp_endpoint.close`; ADRs 0017, 0067,
//! 0089 and 0099; ticket #749).

use std::collections::BTreeMap;
#[expect(
    clippy::disallowed_types,
    reason = "short-lived active lend table lock outside loom tests"
)]
use std::sync::Mutex;
use std::sync::{Arc, PoisonError};

use goodfetch_engine::audit::Origin;
use goodfetch_engine::governor::PageHold;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::audit::{Actor, LENT_PAGE_ATTACHED};
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::lend::{
    CDP_ENDPOINT_CLOSE, CdpEndpointCloseParams, CdpEndpointClosed, CdpEndpointId,
    CdpEndpointOpened, PAGE_LEND, PageLendParams,
};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use jiff::Timestamp;
use serde_json::json;

use super::{Pages, in_call};
use crate::access_mode::BrowserFamily;
use crate::cdp::driver::OtherCodeGuard;
use crate::lending::{self, LendEndpoint, LendStart};
use crate::pool::Hold;
use crate::sessions::OpenPage;
use crate::trace;

/// Active lending endpoints across open pages.
#[derive(Clone, Default)]
pub(super) struct ActiveLends {
    #[expect(
        clippy::disallowed_types,
        reason = "short-lived active lend table lock outside loom tests"
    )]
    inner: Arc<Mutex<LendsState>>,
}

#[derive(Default)]
struct LendsState {
    next_id: u64,
    lends: BTreeMap<CdpEndpointId, Arc<OpenLend>>,
}

struct OpenLend {
    id: CdpEndpointId,
    page_id: PageId,
    page: OpenPage,
    endpoint: LendEndpoint,
    /// Marks the page as readable by external code while lent so Secrets are
    /// refused and results are marked unguarded (ADRs 0089, 0092).
    _other_code: OtherCodeGuard,
    /// Holds the page's browser for area 3's pool while lent.
    _pool_hold: Hold,
}

impl ActiveLends {
    fn next_id(&self, page_id: &PageId) -> CdpEndpointId {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        state.next_id += 1;
        CdpEndpointId(format!("{}.lend-{}", page_id.0, state.next_id))
    }

    fn insert(&self, open: Arc<OpenLend>) {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        state.lends.insert(open.id.clone(), open);
    }

    fn remove(&self, id: &CdpEndpointId) -> Option<Arc<OpenLend>> {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        state.lends.remove(id)
    }
}

/// Adds the handlers of `page.lend` and `cdp_endpoint.close`.
pub(super) fn with_lend_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (lending, closing) = (pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_LEND,
            move |params: PageLendParams, context: CallContext| {
                let pages = lending.clone();
                in_call((PAGE_LEND.name, context.form()), async move {
                    pages.lend_page(params).await
                })
            },
        ))
        .with_handler(Handler::new(
            &CDP_ENDPOINT_CLOSE,
            move |params: CdpEndpointCloseParams, context: CallContext| {
                let pages = closing.clone();
                in_call((CDP_ENDPOINT_CLOSE.name, context.form()), async move {
                    Ok(pages.close_lend(&params.cdp_endpoint_id, "closed").await)
                })
            },
        ))
}

impl Pages {
    /// Opens a scoped WebSocket CDP endpoint for `params.page_id` (ADR 0089).
    async fn lend_page(&self, params: PageLendParams) -> Result<CdpEndpointOpened, CallError> {
        let page = self.sessions.page(&params.page_id).await?;
        check_lend_chromium(&page)?;

        // Effect window counts bodies as sent from the start of page.lend (ADR 0106, #749).
        page.sending(PAGE_LEND.name).await?;

        // Transition Session Open -> Lent (checks stealth.allow_runtime and
        // refuses second lend with `session_reserved`).
        self.sessions.begin_lend(&page.session)?;

        let cdp_endpoint_id = self.lends.next_id(&params.page_id);
        let shared_host = false;

        let cleanup_pages = self.clone();
        let cleanup_id = cdp_endpoint_id.clone();
        let cleanup_owner = page.owner.clone();
        let on_end: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new(move |reason| {
            let pages = cleanup_pages.clone();
            let id = cleanup_id.clone();
            cleanup_owner.tasks().spawn(async move {
                pages.close_lend(&id, reason).await;
            });
        });

        let endpoint = match lending::start(LendStart {
            endpoint_id: cdp_endpoint_id.0.clone(),
            page_id: params.page_id.clone(),
            page: page.clone(),
            shared_host,
            on_end,
        })
        .await
        {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.sessions.end_lend(&page.session, &page.driver).await;
                return Err(error);
            }
        };

        let url = endpoint.url.clone();
        let headers = endpoint.headers.clone();
        let open = Arc::new(OpenLend {
            id: cdp_endpoint_id.clone(),
            page_id: params.page_id.clone(),
            page: page.clone(),
            endpoint,
            _other_code: page.driver.attach_other_code(),
            _pool_hold: page.hold(PageHold::Held),
        });
        self.lends.insert(Arc::clone(&open));

        record_lend_audit(&page, &params.page_id, &cdp_endpoint_id).await;
        trace::provenance(
            &page.run.trace,
            json!({
                "action": "lent_page_attached",
                "unguarded": true,
                "page_id": params.page_id,
                "session_id": page.session,
                "cdp_endpoint_id": cdp_endpoint_id,
            }),
        )
        .await;

        // End the lend automatically if its page closes or detaches.
        let watch_pages = self.clone();
        let watch_id = cdp_endpoint_id.clone();
        let mut events = page.driver.events();
        page.owner.tasks().spawn(async move {
            while events.next().await.is_some() {}
            watch_pages.close_lend(&watch_id, "page_closed").await;
        });

        Ok(CdpEndpointOpened {
            cdp_endpoint_id,
            url,
            headers,
        })
    }

    /// Ends an active lend, revokes its token, re-applies Session state,
    /// releases external-reader/pool guards, and returns the Session to `Open`.
    async fn close_lend(&self, id: &CdpEndpointId, reason: &str) -> CdpEndpointClosed {
        let Some(open) = self.lends.remove(id) else {
            return CdpEndpointClosed {};
        };
        open.endpoint.shutdown();
        self.sessions
            .end_lend(&open.page.session, &open.page.driver)
            .await;
        trace::provenance(
            &open.page.run.trace,
            json!({
                "action": "lent_page_ended",
                "page_id": open.page_id,
                "session_id": open.page.session,
                "cdp_endpoint_id": open.id,
                "reason": reason,
            }),
        )
        .await;
        drop(open);
        CdpEndpointClosed {}
    }
}

fn check_lend_chromium(page: &OpenPage) -> Result<(), CallError> {
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
            "lending a page (`page.lend()`) is only available on Chromium, not Firefox or WebKit (ADR 0089)",
            &CapabilityUnavailableDetails {
                key: PAGE_LEND.name.to_owned(),
                layer: SettingsLayer::Run,
                kind: kind.to_owned(),
                fix: "use a Chromium browser to lend a page".to_owned(),
            },
        ));
    }
    Ok(())
}

async fn record_lend_audit(page: &OpenPage, page_id: &PageId, cdp_endpoint_id: &CdpEndpointId) {
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
        "cdp_endpoint_id": cdp_endpoint_id,
    });
    if let Err(error) = page
        .run
        .engine
        .audit()
        .record(&origin, &LENT_PAGE_ATTACHED, details, now)
        .await
    {
        tracing::warn!(%error, page_id = %page_id.0, "could not record Lent page attach in Audit log");
    }
}
