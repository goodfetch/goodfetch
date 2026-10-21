//! The page API's operations, the formats of a page's document among them
//! (ADR 0007), and the engine's pages (ADRs 0085 and 0092): each live Run's
//! default Session, opened by its first `run.new_page` and shared by every
//! later one, and each open page by its ID. A Run's pages and Session leave
//! with it: when it stops, its browser closes and its entries go.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::runs::{Run, RunError};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_extractor::{Context, Document};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::{
    FormatsParams, GotoParams, Navigated, NewPageParams, PAGE_FORMATS, PAGE_GO_BACK,
    PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageFormats, PageId, PageOpened, PageParams,
    RUN_NEW_PAGE, SessionId,
};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use serde_json::json;
use tokio::sync::Mutex;

use crate::driver::{Destination, LocalBrowser, PageDriver, SessionDriver};

/// A navigation's deadline when the Run's settings name none: 30 s.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

/// What the page crate adds to the engine (ADR 0087): the handlers of
/// `run.new_page`, of the navigations `page.goto`, `page.reload`,
/// `page.go_back` and `page.go_forward`, and of `page.formats`, whose
/// browsers come from `pins`, the builds the Product version pins.
pub fn contribution(pins: Vec<PinnedArtifact>) -> Contribution {
    let pages = Pages {
        state: Arc::new(Mutex::new(State::default())),
        pins,
    };
    let (opening, going, reloading, going_back, going_forward) = (
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
    );
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
                let pages = going.clone();
                async move {
                    pages
                        .navigate(&params.page_id, Destination::Url(&params.url))
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_RELOAD,
            move |params: PageParams, _context: CallContext| {
                let pages = reloading.clone();
                async move { pages.navigate(&params.page_id, Destination::Reload).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GO_BACK,
            move |params: PageParams, _context: CallContext| {
                let pages = going_back.clone();
                async move { pages.navigate(&params.page_id, Destination::Back).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GO_FORWARD,
            move |params: PageParams, _context: CallContext| {
                let pages = going_forward.clone();
                async move { pages.navigate(&params.page_id, Destination::Forward).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_FORMATS,
            move |params: FormatsParams, _context: CallContext| {
                let pages = pages.clone();
                async move { pages.formats(params).await }
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
    run: Run,
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
                run: run.clone(),
                driver,
            },
        );
        Ok(PageOpened {
            page_id: page,
            session_id: run_pages.session.clone(),
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
            state.pages.retain(|_, page| page.run.id() != &run_id);
        });
        opened
    }

    /// The open page `page_id`; `closed` when no page has that ID.
    async fn open(&self, page_id: &PageId) -> Result<OpenPage, CallError> {
        self.state
            .lock()
            .await
            .pages
            .get(page_id)
            .cloned()
            .ok_or_else(|| {
                CallError::new(
                    &CLOSED,
                    format!("the page `{}` is closed, or its Run ended", page_id.0),
                    &json!({ "page_id": page_id }),
                )
            })
    }

    /// Navigates the page `page_id` to `destination`.
    async fn navigate(
        &self,
        page_id: &PageId,
        destination: Destination<'_>,
    ) -> Result<Navigated, CallError> {
        let page = self.open(page_id).await?;
        let response = page.driver.navigate(destination).await?;
        Ok(Navigated { response })
    }

    /// The formats the request names, read from one serialization of the
    /// page's document on a blocking thread of its Run's.
    async fn formats(&self, params: FormatsParams) -> Result<PageFormats, CallError> {
        let page = self.open(&params.page_id).await?;
        let document = page.driver.document().await?;
        let request = params.formats;
        Ok(page
            .run
            .owner()
            .blocking(move || {
                goodfetch_extractor::formats(
                    &Document {
                        html: &document.html,
                        url: &document.url,
                        base_url: &document.base_url,
                    },
                    &request,
                    &Context::default(),
                )
            })
            .await)
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
