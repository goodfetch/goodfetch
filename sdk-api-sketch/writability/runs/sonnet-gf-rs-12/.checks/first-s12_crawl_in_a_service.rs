//! A small web service with one endpoint, `GET /books?category=<name>`, that crawls that category
//! of <https://books.toscrape.com/> (following its "next" pages) when called and answers with its
//! books as JSON (title and price).
//!
//! * Every call starts a Run of its own on the one shared Foxhound connection, so two calls at the
//!   same time do not interfere, and a later call works the same.
//! * When the caller disconnects, axum drops the handler's future; its guard aborts the crawl, and
//!   the crawl's thread closes the Run, which releases the Run's Sessions and browsers.
//! * The service prints nothing to standard output; its own log lines go to standard error. The
//!   library's logs are `tracing` events and the library installs no subscriber, so they go to
//!   whatever subscriber the service installs (this package depends on none).
//! * The same crawl runs as a plain script: `s12_crawl_in_a_service crawl <category>` prints the
//!   books as JSON, the only place this program writes to standard output.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::future::{AbortHandle, AbortRegistration, Abortable};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRun, Dataset, Error, Foxhound, Label, Seed,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The site the crawl starts from.
const SITE: &str = "https://books.toscrape.com/";
/// How long one crawl may last.
const CRAWL_TIMEOUT: Duration = Duration::from_secs(300);

/// One book, as the Dataset holds it and the endpoint answers it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Book {
    /// Its title.
    pub title: String,
    /// Its price, as the site shows it.
    pub price: String,
}

/// The query of `/books`.
#[derive(Debug, Deserialize)]
struct Params {
    category: String,
}

/// The answer of `/books`.
#[derive(Debug, Serialize)]
struct Reply {
    category: String,
    books: Vec<Book>,
}

/// Why a crawl gave no books.
#[derive(Debug)]
enum Failure {
    /// The caller went away and the crawl was stopped.
    Cancelled,
    /// The crawl or the engine failed.
    Failed(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("the crawl was cancelled"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Failed(error.to_string())
    }
}

/// The service's own logger: standard error, never standard output.
fn log(message: impl fmt::Display) {
    eprintln!("[books-service] {message}");
}

/// Crawls one category and returns its books, or `None` when the site has no such category.
///
/// The Run is put in `slot` as soon as it exists, so the caller can close it however this future
/// ends, dropped half-way included.
async fn crawl_books(
    fox: &Foxhound,
    category: &str,
    slot: &RefCell<Option<CrawlRun>>,
) -> Result<Option<Vec<Book>>, Failure> {
    let category_missing = Cell::new(false);
    let missing = &category_missing;

    let mut crawl = Crawl::new();
    let books: Dataset<Book> = crawl.dataset("books")?;
    let home: Label<String> = crawl.label("home")?;
    let listing: Label<()> = crawl.label("listing")?;

    // The home page: find the category's link in the side list and follow it.
    crawl.handle(&home, {
        let listing = listing.clone();
        async move |ctx: CrawlContext<String>| -> Result<(), Error> {
            let wanted = ctx.request().payload().trim().to_lowercase();
            let links = ctx.page().locator(".side_categories ul ul a");
            let names = links.all_text_contents().await?;
            let Some(index) = names.iter().position(|name| name.trim().to_lowercase() == wanted)
            else {
                missing.set(true);
                return Ok(());
            };
            ctx.enqueue_links(&links.nth(index), &listing, ()).await?;
            Ok(())
        }
    })?;

    // A listing page: record its books, then follow its "next" link.
    crawl.handle(&listing, {
        let listing = listing.clone();
        let books = books.clone();
        async move |ctx: CrawlContext<()>| -> Result<(), Error> {
            let page = ctx.page();
            for card in page.locator("article.product_pod").all().await? {
                let title = card.locator("h3 a").get_attribute("title").await?.unwrap_or_default();
                let price = card.locator(".price_color").text_content().await?;
                ctx.push(&books, &Book { title, price: price.trim().to_owned() }).await?;
            }
            ctx.enqueue_links(&page.locator("li.next a"), &listing, ()).await?;
            Ok(())
        }
    })?;

    let seed = Seed::new(SITE, &home, category.to_owned());
    let run = fox.start_crawl(&crawl, [seed]).timeout(CRAWL_TIMEOUT).await?;
    *slot.borrow_mut() = Some(run.clone());

    let report = crawl.serve(&run).await?;
    if category_missing.get() {
        return Ok(None);
    }
    let requests = &report.requests;
    if requests.failed > 0 || requests.blocked > 0 || requests.pending > 0 {
        return Err(Failure::Failed(format!(
            "the crawl ended incomplete: {} failed, {} blocked, {} pending",
            requests.failed, requests.blocked, requests.pending
        )));
    }

    let mut found = Vec::new();
    let mut records = run.records(&books);
    while let Some(record) = records.next().await {
        found.push(record?.value);
    }
    Ok(Some(found))
}

/// Runs the crawl until it ends or `stop` aborts it, then closes its Run on every path.
async fn run_crawl(
    fox: &Foxhound,
    category: &str,
    stop: AbortRegistration,
) -> Result<Option<Vec<Book>>, Failure> {
    let slot = RefCell::new(None);
    let result = Abortable::new(crawl_books(fox, category, &slot), stop).await;
    let run = slot.take();
    if let Some(run) = run {
        if let Err(error) = run.as_run().close().await {
            log(format_args!("closing the Run of {category:?} failed: {error}"));
        }
    }
    match result {
        Ok(outcome) => outcome,
        Err(_) => Err(Failure::Cancelled),
    }
}

/// Stops its crawl when dropped: the handler's future is dropped when the caller disconnects.
struct StopOnDrop(AbortHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs one crawl on a thread of its own with a single-threaded runtime: a crawl's handlers run
/// inside the future that serves them and need not be `Send`.
fn crawl_on_own_thread(
    fox: Foxhound,
    category: String,
    stop: AbortRegistration,
) -> Result<Option<Vec<Book>>, Failure> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Failure::Failed(error.to_string()))?;
    runtime.block_on(run_crawl(&fox, &category, stop))
}

/// `GET /books?category=<name>`.
async fn books(State(fox): State<Foxhound>, Query(params): Query<Params>) -> Response {
    let category = params.category;
    log(format_args!("crawling category {category:?}"));

    let (handle, registration) = AbortHandle::new_pair();
    let _stop = StopOnDrop(handle);
    let crawl = {
        let category = category.clone();
        tokio::task::spawn_blocking(move || crawl_on_own_thread(fox, category, registration))
    };

    let answer = match crawl.await {
        Ok(Ok(Some(books))) => {
            log(format_args!("category {category:?}: {} books", books.len()));
            Json(Reply { category, books }).into_response()
        }
        Ok(Ok(None)) => {
            log(format_args!("category {category:?}: not found"));
            error_reply(StatusCode::NOT_FOUND, format!("no category named {category:?}"))
        }
        Ok(Err(failure)) => {
            log(format_args!("category {category:?}: {failure}"));
            let status = match failure {
                Failure::Cancelled => StatusCode::SERVICE_UNAVAILABLE,
                Failure::Failed(_) => StatusCode::BAD_GATEWAY,
            };
            error_reply(status, failure.to_string())
        }
        Err(error) => {
            log(format_args!("category {category:?}: crawl thread failed: {error}"));
            error_reply(StatusCode::INTERNAL_SERVER_ERROR, "the crawl thread failed".to_owned())
        }
    };
    answer
}

/// A JSON error answer.
fn error_reply(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Serves `/books` until Ctrl-C.
async fn serve(fox: Foxhound) -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new().route("/books", get(books)).with_state(fox);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    log("listening on http://127.0.0.1:3000");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// The same crawl from a plain script: prints the category's books as JSON.
async fn script(fox: &Foxhound, category: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (_handle, registration) = AbortHandle::new_pair();
    match run_crawl(fox, category, registration).await {
        Ok(Some(books)) => {
            println!("{}", serde_json::to_string_pretty(&books)?);
            Ok(())
        }
        Ok(None) => Err(format!("no category named {category:?}").into()),
        Err(failure) => Err(failure.to_string().into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let category = args.next();

    // No project file: the service's working directory means nothing to it.
    let fox = Foxhound::connect(ConnectOptions::default().no_project()).await?;
    let outcome = match (command.as_deref(), category) {
        (Some("crawl"), Some(category)) => script(&fox, &category).await,
        _ => serve(fox.clone()).await,
    };
    let closed = fox.close().await;
    outcome?;
    closed?;
    Ok(())
}
