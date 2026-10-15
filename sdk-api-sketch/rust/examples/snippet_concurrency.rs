//! Snippet for Concurrency model per language (#468), and Send bounds on a Rust crawl's handlers
//! (#531, ADR 0112).

use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::time::Duration;

use futures_util::TryStreamExt;
use futures_util::future::{Either, select};
use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRun, CrawlRunReport, Dataset, Error, Foxhound, Page, Seed, UrlMatcher, blocking,
    events,
};
use serde::{Deserialize, Serialize};

/// Handles are cheap clones and a call owns what it needs, so its future can be spawned on any
/// executor (Q4).
fn spawnable(page: &Page) -> impl Future<Output = Result<(), Error>> + Send + 'static {
    let page = page.clone();
    async move { page.locator("li.next a").click().await }
}

/// The async API spawns nothing on the program's executor: what the engine sends on its own
/// arrives as a stream the program drives (Q5, Q8).
async fn browse(page: &Page) -> Result<String, Error> {
    // Subscribed when called, so nothing slips through before the first read.
    let mut console = page.events(events::Console);
    // Routes are answered here, inside the future this function awaits, up to eight at once.
    let answer_routes = page
        .route("**/*.{png,jpg}")
        .try_for_each_concurrent(8, async |route| route.abort().await);
    let navigate = async {
        page.goto("https://books.toscrape.com/").await?;
        // A regular expression runs in the engine; a lookahead such as `(?=` fails at the call.
        let next_page = UrlMatcher::Regex(r"/catalogue/page-\d+\.html$".to_owned());
        page.wait_for_response(next_page)
            .during(page.locator("li.next a").click())
            .await
    };
    // Once the navigation ends, the route future is dropped, and with it the subscription.
    match select(pin!(navigate), pin!(answer_routes)).await {
        Either::Left((navigated, _)) => drop(navigated?),
        Either::Right((answered, _)) => answered?,
    }

    // A predicate runs in the program and may use its own values (P22).
    let seen: HashSet<String> = HashSet::new();
    let (fresh, ()) = page
        .wait_for_event(events::Response)
        .matching(move |response| !seen.contains(response.url()))
        .timeout(Duration::from_secs(5))
        .during(page.locator("li.next a").click())
        .await?;
    let first_log = match console.next().await {
        Some(message) => message?.text,
        None => String::new(),
    };
    Ok(format!("{} {first_log}", fresh.url()))
}

/// A book's title, as a crawl's Dataset holds it.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Title {
    title: String,
}

/// A sequence read to its end; dropping the stream unsubscribes (Q10). A crawl's Records are typed
/// by their Dataset and read from its Run (ADR 0103).
async fn titles(run: &CrawlRun, books: &Dataset<Title>, limit: usize) -> Result<Vec<String>, Error> {
    let mut records = run.records(books);
    let mut titles = Vec::new();
    while let Some(record) = records.next().await {
        titles.push(record?.value.title);
        if titles.len() >= limit {
            break;
        }
    }
    Ok(titles)
}

/// A crawl served on a task of its own (ADR 0112). Each handler is cloned for each request, so an
/// `async move` closure owns clones of the handles it uses and the `Crawl` is `'static`; handlers
/// and their futures are `Send`, so `serve`'s future is too, and Tokio's multi-threaded runtime
/// spawns it, as axum runs it in scenario 12.
async fn spawned(foxhound: &Foxhound) -> Result<CrawlRunReport, Box<dyn std::error::Error + Send + Sync>> {
    let mut crawl = Crawl::new();
    let titles = crawl.dataset::<Title>("titles")?;
    let book = crawl.label::<()>("book")?;
    crawl.handle(&book, async move |ctx: CrawlContext<()>| {
        let title = ctx.page().locator("h1").text_content().await?;
        ctx.push(&titles, &Title { title }).await
    })?;
    let seed = Seed::new("https://books.toscrape.com/catalogue/sharp-objects_997/index.html", &book, ());
    let run = foxhound.start_crawl(&crawl, [seed]).await?;
    let serving = tokio::spawn(async move { crawl.serve(&run).await });
    Ok(serving.await??)
}

/// State that is not `Send` stays in the program, which answers the crawl requests itself: the
/// stream beneath `serve` takes no closure and asks no bound (ADR 0112, G8).
async fn answered_here(run: &CrawlRun) -> Result<usize, Error> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut calls = run.crawl_requests();
    while let Some(call) = calls.next().await {
        let call = call?;
        seen.borrow_mut().push(call.context().request().url().to_owned());
        call.finish(Ok(())).await?;
    }
    Ok(seen.borrow().len())
}

/// The same API without `async`, for programs that have no runtime (Q7).
fn script() -> Result<usize, Error> {
    let foxhound = blocking::Foxhound::connect(ConnectOptions::default())?;
    let run = foxhound.start().send()?;
    let page = run.new_page().send()?;
    let console = page.events(events::Console); // an Iterator, subscribed when called
    page.goto("https://books.toscrape.com/")
        .timeout(Duration::from_secs(10))
        .send()?;
    page.close().send()?; // ends the sequence
    let count = console.take(20).count();
    foxhound.close().send()?;
    Ok(count)
}

fn main() {
    let _ = (spawnable, browse, titles, spawned, answered_here, script);
}
