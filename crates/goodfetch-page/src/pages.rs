//! The page API's operations, the formats of a page's document among them
//! (ADR 0007), and the engine's pages (ADRs 0085 and 0092): each live Run's
//! default Session, opened by its first `run.new_page` and shared by every
//! later one, and each open page by its ID. A Run's pages and Session leave
//! with it: when it stops, its browser closes and its entries go. A locator
//! action reaches its page's driver through the guard, under the Run's
//! `timeout.action`. A navigation the browser turns into a download answers
//! no response and publishes `page.download` on the engine's event stream.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::{PinnedArtifact, Pins};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::events::Events;
use goodfetch_engine::runs::{Run, RunError};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_extractor::{Context, Document};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::{
    Acted, ClickParams, FillParams, FormatsParams, GotoParams, LOCATOR_CLICK, LOCATOR_FILL,
    LOCATOR_PRESS, Locator, Navigated, NewPageParams, PAGE_DOWNLOAD, PAGE_FORMATS, PAGE_GO_BACK,
    PAGE_GO_FORWARD, PAGE_GOTO, PAGE_RELOAD, PageDownload, PageFormats, PageId, PageOpened,
    PageParams, PressParams, RUN_NEW_PAGE, SessionId, WaitUntil,
};
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId};
use goodfetch_protocol::settings::{
    SettingLimit, SettingsLayer, TIMEOUT_ACTION, TIMEOUT_NAVIGATION,
};
use serde_json::json;
use tokio::sync::Mutex;

use crate::driver::{Arrived, Destination, LocalBrowser, PageDriver, SessionDriver};
use crate::guard::{self, Action, Request, Window};

/// A navigation's deadline when the Run's settings name none: 30 s.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);

/// The guard's window when the Run's settings name none: 30 s, the
/// setting's default.
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// What the page crate adds to the engine (ADR 0087): the handlers of
/// `run.new_page`, of the navigations `page.goto`, `page.reload`,
/// `page.go_back` and `page.go_forward`, of `page.formats` and of the
/// locator actions, whose browsers come from `pins`, the builds the Product
/// version pins.
pub fn contribution(pins: Vec<PinnedArtifact>) -> Contribution {
    let pages = Pages {
        state: Arc::new(Mutex::new(State::default())),
        pins,
    };
    let contribution = with_page_handlers(Contribution::new("page"), &pages);
    with_locator_handlers(contribution, &pages)
}

/// Adds the handlers of `run.new_page`, the navigations and `page.formats`.
fn with_page_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (opening, going, reloading, going_back, going_forward, formatting) = (
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
        pages.clone(),
    );
    contribution
        .with_handler(Handler::new(
            &RUN_NEW_PAGE,
            move |params: NewPageParams, context: CallContext| {
                let pages = opening.clone();
                async move { pages.new_page(params, context).await }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GOTO,
            move |params: GotoParams, context: CallContext| {
                let pages = going.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Url(&params.url),
                            params.wait_until.unwrap_or_default(),
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_RELOAD,
            move |params: PageParams, context: CallContext| {
                let pages = reloading.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Reload,
                            WaitUntil::Load,
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GO_BACK,
            move |params: PageParams, context: CallContext| {
                let pages = going_back.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Back,
                            WaitUntil::Load,
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_GO_FORWARD,
            move |params: PageParams, context: CallContext| {
                let pages = going_forward.clone();
                async move {
                    pages
                        .navigate(
                            &params.page_id,
                            Destination::Forward,
                            WaitUntil::Load,
                            context.events(),
                        )
                        .await
                }
            },
        ))
        .with_handler(Handler::new(
            &PAGE_FORMATS,
            move |params: FormatsParams, _context: CallContext| {
                let pages = formatting.clone();
                async move { pages.formats(params).await }
            },
        ))
}

/// Adds the handlers of the locator actions.
fn with_locator_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (clicking, filling, pressing) = (pages.clone(), pages.clone(), pages.clone());
    contribution
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
                let pages = pressing.clone();
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
    run: Run,
    driver: PageDriver,
    /// Its Run's `timeout.action` and where it was set.
    action_timeout: (SettingLimit, Duration),
    /// Its Run's clock.
    clock: Clock,
}

impl Pages {
    #[tracing::instrument(name = "page", skip_all, fields(run_id = %params.run_id, page_id = tracing::field::Empty, session_id = tracing::field::Empty))]
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
        let span = tracing::Span::current();
        span.record("page_id", page.0.as_str());
        span.record("session_id", run_pages.session.0.as_str());
        state.pages.insert(
            page.clone(),
            OpenPage {
                run: run.clone(),
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
            state.pages.retain(|_, page| page.run.id() != &run_id);
        });
        opened
    }

    /// Navigates the page `page_id` to `destination`, answering once its
    /// document reached the state `wait_until` names; a download it turns
    /// into is published on `events`.
    #[tracing::instrument(name = "page", skip_all, fields(page_id = %page_id.0, operation = destination.operation(), run_id = tracing::field::Empty, url.origin = tracing::field::Empty))]
    async fn navigate(
        &self,
        page_id: &PageId,
        destination: Destination<'_>,
        wait_until: WaitUntil,
        events: &Events,
    ) -> Result<Navigated, CallError> {
        let span = tracing::Span::current();
        if let Destination::Url(url) = destination {
            span.record("url.origin", goodfetch_engine::log::origin(url));
        }
        let page = self.open(page_id).await?;
        span.record("run_id", page.run.id().as_str());
        match page.driver.navigate(destination, wait_until).await? {
            Arrived::Document(response) => Ok(Navigated { response }),
            Arrived::Download {
                url,
                suggested_filename,
            } => {
                events
                    .publish(
                        &PAGE_DOWNLOAD,
                        Some(&page_id.0),
                        &PageDownload {
                            page_id: page_id.clone(),
                            url,
                            suggested_filename,
                        },
                    )
                    .await;
                Ok(Navigated { response: None })
            }
        }
    }

    /// Performs `action` on the element `locator` matches, through the
    /// guard; `action` is the params' own failure when they name none.
    #[tracing::instrument(name = "page", skip_all, fields(page_id = %locator.page_id.0, operation, run_id = tracing::field::Empty))]
    async fn act(
        &self,
        operation: &'static str,
        locator: Locator,
        action: Result<Action, CallError>,
        context: &CallContext,
    ) -> Result<Acted, CallError> {
        let action = action?;
        let page = self.open(&locator.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
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

    /// The formats the request names, read from one serialization of the
    /// page's document on a blocking thread of its Run's.
    #[tracing::instrument(name = "page", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_FORMATS.name, run_id = tracing::field::Empty))]
    async fn formats(&self, params: FormatsParams) -> Result<PageFormats, CallError> {
        let page = self.open(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.id().as_str());
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
