//! Snippet for Concurrency model per language (#468).

use std::collections::HashSet;
use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use futures_util::TryStreamExt;
use futures_util::future::{Either, select};
use goodfetch_sdk_sketch::{ConnectOptions, Error, Page, Run, UrlMatcher, blocking, events};

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

/// A sequence read to its end; dropping the stream unsubscribes (Q10).
async fn titles(run: &Run, limit: usize) -> Result<Vec<String>, Error> {
    let mut records = run.records();
    let mut titles = Vec::new();
    while let Some(record) = records.next().await {
        if let Some(title) = record?.text("title") {
            titles.push(title.to_owned());
        }
        if titles.len() >= limit {
            break;
        }
    }
    Ok(titles)
}

/// The same API without `async`, for programs that have no runtime (Q7).
fn script() -> Result<usize, Error> {
    let gf = blocking::Goodfetch::connect(ConnectOptions::default())?;
    let run = gf.collie().start().send()?;
    let page = run.new_page().send()?;
    page.goto("https://books.toscrape.com/")
        .timeout(Duration::from_secs(10))
        .send()?;
    let count = run.records().take(20).count();
    gf.close().send()?;
    Ok(count)
}

fn main() {
    let _ = (spawnable, browse, titles, script);
}
