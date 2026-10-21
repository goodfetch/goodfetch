//! The page API's operations and the engine's pages (ADRs 0085 and 0092):
//! each live Run's default Session, opened by its first `run.new_page` and
//! shared by every later one, and each open page by its ID. A Run's pages
//! and Session leave with it: when it stops, its browser closes and its
//! entries go. A locator action reaches its page's driver through the
//! guard, under the Run's `timeout.action`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::runs::{Run, RunError};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::{
    Acted, ClickParams, FillParams, GotoParams, LOCATOR_CLICK, LOCATOR_FILL, LOCATOR_PRESS,
    Locator, Navigated, NewPageParams, PAGE_GOTO, PageId, PageOpened, PressParams, RUN_NEW_PAGE,
    SessionId,
};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::settings::{
    SettingLimit, SettingsLayer, TIMEOUT_ACTION, TIMEOUT_NAVIGATION,
};
use serde_json::json;
use tokio::sync::Mutex;

use crate::driver::{LocalBrowser, PageDriver, SessionDriver};
use crate::guard::{self, Action, Request, Window};

/// A navigation's deadline when the Run's settings name none: 30 s.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

/// The guard's window when the Run's settings name none: 30 s, the
/// setting's default.
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// What the page crate adds to the engine (ADR 0087): the handlers of
/// `run.new_page`, `page.goto` and the locator actions, whose browsers come
/// from `pins`, the builds the Product version pins.
pub fn contribution(pins: Vec<PinnedArtifact>) -> Contribution {
    let pages = Pages {
        state: Arc::new(Mutex::new(State::default())),
        pins,
    };
    let opening = pages.clone();
    let going = pages.clone();
    let clicking = pages.clone();
    let filling = pages.clone();
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
                async move { pages.goto(params).await }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_CLICK,
            move |params: ClickParams, context: CallContext| {
                let pages = clicking.clone();
                async move {
                    pages
                        .act(
                            LOCATOR_CLICK.name,
                            params.locator,
                            Ok(Action::Click),
                            &context,
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_FILL,
            move |params: FillParams, context: CallContext| {
                let pages = filling.clone();
                async move {
                    pages
                        .act(
                            LOCATOR_FILL.name,
                            params.locator,
                            Ok(Action::Fill(params.value)),
                            &context,
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &LOCATOR_PRESS,
            move |params: PressParams, context: CallContext| {
                let pages = pages.clone();
                async move {
                    let action = Action::press(&params.key);
                    pages
                        .act(LOCATOR_PRESS.name, params.locator, action, &context)
                        .await
                }
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
    /// Its Run's `timeout.action` and where it was set.
    action_timeout: (SettingLimit, Duration),
    /// Its Run's clock.
    clock: Clock,
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
        let action_timeout = run
            .settings()
            .and_then(|settings| Deadline::of(settings.resolved(), &TIMEOUT_ACTION))
            .map_or_else(
                || {
                    let limit = SettingLimit {
                        key: TIMEOUT_ACTION.name.to_owned(),
                        value: json!(ACTION_TIMEOUT.as_millis()),
                        layer: SettingsLayer::Default,
                    };
                    (limit, ACTION_TIMEOUT)
                },
                |deadline| (deadline.limit().clone(), deadline.duration()),
            );
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
                action_timeout,
                clock: run.clock(),
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
            state.pages.retain(|_, page| page.run != run_id);
        });
        opened
    }

    async fn goto(&self, params: GotoParams) -> Result<Navigated, CallError> {
        let page = self.open(&params.page_id).await?;
        let response = page.driver.goto(&params.url).await?;
        Ok(Navigated { response })
    }

    /// Performs `action` on the element `locator` matches, through the
    /// guard; `action` is the params' own failure when they name none.
    async fn act(
        &self,
        operation: &'static str,
        locator: Locator,
        action: Result<Action, CallError>,
        context: &CallContext,
    ) -> Result<Acted, CallError> {
        let action = action?;
        let page = self.open(&locator.page_id).await?;
        let (limit, duration) = page.action_timeout.clone();
        let request = Request {
            operation,
            selector: &locator.selector,
            action,
            window: Window { limit, duration },
            surface: context.surface(),
        };
        guard::act(&page.driver, request, &page.clock).await?;
        Ok(Acted {})
    }

    /// The open page `page_id`, or `closed`.
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
