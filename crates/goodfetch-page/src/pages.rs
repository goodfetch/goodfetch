//! The page API's operations and the engine's pages (ADRs 0085 and 0092):
//! each live Run's default Session, opened by its first `run.new_page` and
//! shared by every later one, and each open page by its ID. A Run's pages
//! and Session leave with it: when it stops, its browser closes and its
//! entries go.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::runs::{Run, RunError};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::{
    GotoParams, Navigated, NewPageParams, PAGE_GOTO, PageId, PageOpened, RUN_NEW_PAGE, SessionId,
};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use serde_json::json;
use tokio::sync::Mutex;

use crate::driver::{LocalBrowser, PageDriver, SessionDriver};

/// A navigation's deadline when the Run's settings name none: 30 s.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

/// What the page crate adds to the engine (ADR 0087): the handlers of
/// `run.new_page` and `page.goto`, whose browsers come from `pins`, the
/// builds the Product version pins.
pub fn contribution(pins: Vec<PinnedArtifact>) -> Contribution {
    let pages = Pages {
        state: Arc::new(Mutex::new(State::default())),
        pins,
    };
    let opening = pages.clone();
    Contribution::new("page")
        .with_handler(Handler::new(
            &RUN_NEW_PAGE,
            move |params: NewPageParams, context: CallContext| {
                let pages = opening.clone();
                async move { pages.new_page(params, context).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GOTO,
            move |params: GotoParams, _context: CallContext| {
                let pages = pages.clone();
                async move { pages.goto(params).await }
            },
        ))
}

/// The engine's pages, which every call of every Connection shares.
#[derive(Clone)]
struct Pages {
    state: Arc<Mutex<State>>,
    pins: Vec<PinnedArtifact>,
}

#[derive(Default)]
struct State {
    runs: BTreeMap<RunId, Arc<RunPages>>,
    pages: BTreeMap<PageId, OpenPage>,
    opened_sessions: u64,
    opened_pages: u64,
}

/// A live Run's default Session.
struct RunPages {
    session: SessionId,
    driver: SessionDriver<LocalBrowser>,
}

#[derive(Clone)]
struct OpenPage {
    run: RunId,
    driver: PageDriver,
}

impl Pages {
    async fn new_page(
        &self,
        params: NewPageParams,
        context: CallContext,
    ) -> Result<PageOpened, CallError> {
        let run = context
            .runs()
            .live(params.run_id.clone())
            .await
            .map_err(|error| run_error(&error, &params.run_id))?;
        let navigation_timeout = run
            .settings()
            .and_then(|settings| settings.resolved().duration(TIMEOUT_NAVIGATION.name))
            .unwrap_or(NAVIGATION_TIMEOUT);
        let run_pages = self.default_session(&run, &context).await;
        let driver = run_pages.driver.new_page(navigation_timeout).await?;
        let mut state = self.state.lock().await;
        state.opened_pages += 1;
        let page = PageId(format!("page-{}", state.opened_pages));
        state.pages.insert(
            page.clone(),
            OpenPage {
                run: run.id().clone(),
                driver,
            },
        );
        Ok(PageOpened {
            page,
            session: run_pages.session.clone(),
        })
    }

    /// The Run's default Session, opened now when it has none; its entries
    /// leave when the Run stops.
    async fn default_session(&self, run: &Run, context: &CallContext) -> Arc<RunPages> {
        let mut state = self.state.lock().await;
        if let Some(existing) = state.runs.get(run.id()) {
            return Arc::clone(existing);
        }
        state.opened_sessions += 1;
        let opened = Arc::new(RunPages {
            session: SessionId(format!("session-{}", state.opened_sessions)),
            driver: SessionDriver::new(LocalBrowser::new(
                run.clone(),
                context.runs().home().clone(),
                Pins {
                    pinned_by: PinnedBy {
                        product: context.identity().product,
                        product_version: context.identity().product_version.clone(),
                    },
                    artifacts: self.pins.clone(),
                },
            )),
        });
        state.runs.insert(run.id().clone(), Arc::clone(&opened));
        drop(state);
        let pages = self.clone();
        let stopping = run.cancellation().clone();
        let run_id = run.id().clone();
        run.spawn(async move {
            stopping.cancelled().await;
            let mut state = pages.state.lock().await;
            state.runs.remove(&run_id);
            state.pages.retain(|_, page| page.run != run_id);
        });
        opened
    }

    async fn goto(&self, params: GotoParams) -> Result<Navigated, CallError> {
        let page = self
            .state
            .lock()
            .await
            .pages
            .get(&params.page)
            .cloned()
            .ok_or_else(|| {
                CallError::new(
                    &CLOSED,
                    format!("the page `{}` is closed, or its Run ended", params.page.0),
                    &json!({ "page": params.page }),
                )
            })?;
        let response = page.driver.goto(&params.url).await?;
        Ok(Navigated { response })
    }
}

fn run_error(error: &RunError, run_id: &RunId) -> CallError {
    CallError::new(
        &RUN_NOT_FOUND,
        format!("No live Run has the Run ID `{run_id}`: {error}"),
        &RunDetails {
            run_id: run_id.clone(),
            path: None,
        },
    )
}
