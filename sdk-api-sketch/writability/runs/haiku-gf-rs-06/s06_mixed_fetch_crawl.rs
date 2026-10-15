//! Collects every quote, with its author and tags, from three versions of quotes.toscrape.com:
//! the plain HTML site, the JavaScript-rendered `/js/` site and the infinite-scrolling `/scroll`
//! site. One crawl serves all three. The plain and JavaScript pages share one handler: it runs
//! over plain HTTP, and a wait for quotes that a static document lacks escalates only that page to
//! the browser. The scrolling page is forced to the browser tier, and its quotes come from the JSON
//! responses the page loads.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use futures_util::future::{Either, select};
use goodfetch_foxhound::crawl::{Crawl, CrawlContext, CrawlRun, DatasetDeclaration, LabelDeclaration, Seed};
use goodfetch_foxhound::settings::{Settings, fetch};
use goodfetch_foxhound::{
    AbortReason, ConnectOptions, Dataset, ElementState, Error, FetchTier, Foxhound, Label, Page,
    ResourceType,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The URL pattern of the JSON responses the scrolling page loads.
const QUOTES_API: &str = "**/api/quotes**";
/// How long to wait for each JSON response, and for the page to load it.
const API_WAIT: Duration = Duration::from_secs(60);
/// Scrolls to the end of the page; the page loads its next batch when it gets there.
const SCROLL_TO_END: &str = "(window.scrollTo(0, document.body.scrollHeight), true)";
/// Every quote card of a plain or JavaScript page.
const CARDS: &str = "xpath=//div[@class='quote']";

/// One quote, as the Datasets record it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Quote {
    /// The quote's text.
    text: String,
    /// Its author's name.
    author: String,
    /// Its tags.
    tags: Vec<String>,
}

/// The quotes of the three versions.
struct Versions {
    /// From the plain HTML site.
    plain: Vec<Quote>,
    /// From the JavaScript-rendered site.
    js: Vec<Quote>,
    /// From the infinite-scrolling site.
    scroll: Vec<Quote>,
}

/// The printed result.
#[derive(Serialize)]
struct Output<'a> {
    /// The plain HTML quotes.
    plain: &'a [Quote],
    /// The JavaScript quotes.
    js: &'a [Quote],
    /// The scrolling quotes.
    scroll: &'a [Quote],
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fox = Foxhound::connect(ConnectOptions::default()).await?;
    let crawled = crawl_quotes(&fox).await;
    let closed = fox.close().await;
    let versions = crawled?;
    closed?;

    for (name, quotes) in [("plain", &versions.plain), ("js", &versions.js), ("scroll", &versions.scroll)] {
        require_quotes(name, quotes)?;
    }
    let output = Output {
        plain: versions.plain.as_slice(),
        js: versions.js.as_slice(),
        scroll: versions.scroll.as_slice(),
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Runs the crawl over the three sites and reads back each version's quotes.
async fn crawl_quotes(fox: &Foxhound) -> Result<Versions, Box<dyn std::error::Error>> {
    let mut crawl = Crawl::new();
    let plain_quotes = crawl.dataset(DatasetDeclaration::<Quote>::new("plain_quotes").key(&["text", "author"]))?;
    let js_quotes = crawl.dataset(DatasetDeclaration::<Quote>::new("js_quotes").key(&["text", "author"]))?;
    let scroll_quotes = crawl.dataset(DatasetDeclaration::<Quote>::new("scroll_quotes").key(&["text", "author"]))?;

    let plain_page: Label<()> = crawl.label(LabelDeclaration::<()>::new("plain_page"))?;
    let js_page: Label<()> = crawl.label(LabelDeclaration::<()>::new("js_page"))?;
    let scroll_page: Label<()> = crawl.label(
        LabelDeclaration::<()>::new("scroll_page").settings(
            Settings::new().set(fetch::TIER, fetch::TierChoice::Forced(FetchTier::Browser)),
        ),
    )?;

    crawl.handle(&plain_page, async |ctx| handle_html(ctx, &plain_quotes).await)?;
    crawl.handle(&js_page, async |ctx| handle_html(ctx, &js_quotes).await)?;
    crawl.handle(&scroll_page, async |ctx| handle_scroll(ctx, &scroll_quotes).await)?;

    let seeds = [
        Seed::new("https://quotes.toscrape.com/", &plain_page, ()),
        Seed::new("https://quotes.toscrape.com/js/", &js_page, ()),
        Seed::new("https://quotes.toscrape.com/scroll", &scroll_page, ()),
    ];
    let run = fox.start_crawl(&crawl, seeds).await?;
    let report = crawl.serve(&run).await?;

    let totals = &report.requests;
    let unfinished = totals.failed + totals.skipped + totals.blocked + totals.pending;
    if unfinished > 0 {
        return Err(format!(
            "the crawl did not finish every request: {} failed, {} skipped, {} blocked, {} pending",
            totals.failed, totals.skipped, totals.blocked, totals.pending
        )
        .into());
    }

    Ok(Versions {
        plain: read_quotes(&run, &plain_quotes).await?,
        js: read_quotes(&run, &js_quotes).await?,
        scroll: read_quotes(&run, &scroll_quotes).await?,
    })
}

/// Reads a Dataset's committed Records, each quote once.
async fn read_quotes(run: &CrawlRun, quotes: &Dataset<Quote>) -> Result<Vec<Quote>, Error> {
    let mut records = run.records(quotes);
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    while let Some(record) = records.next().await {
        let quote = record?.value;
        if seen.insert((quote.text.clone(), quote.author.clone())) {
            found.push(quote);
        }
    }
    Ok(found)
}

/// Fails when a version yielded no quotes, so an empty result never reads as success.
fn require_quotes(name: &str, quotes: &[Quote]) -> Result<(), Box<dyn std::error::Error>> {
    if quotes.is_empty() {
        return Err(format!("no quotes were collected from the {name} version").into());
    }
    Ok(())
}

/// The handler of the plain and JavaScript pages. Over HTTP, the wait for a quote card escalates
/// the attempt to the browser when the static document has none.
async fn handle_html(ctx: CrawlContext<()>, quotes: &Dataset<Quote>) -> Result<(), Error> {
    if ctx.tier() == &FetchTier::Browser {
        with_resource_blocking(ctx.page(), extract_html(&ctx, quotes)).await
    } else {
        extract_html(&ctx, quotes).await
    }
}

/// Stages every quote card of the page and enqueues the next page of the same Label.
async fn extract_html(ctx: &CrawlContext<()>, quotes: &Dataset<Quote>) -> Result<(), Error> {
    let page = ctx.page();
    page.locator(CARDS).first().wait_for(ElementState::Attached).await?;
    let count = page.locator(CARDS).count().await?;
    if count == 0 {
        return Err(ctx.retry("the page has no quote cards"));
    }
    for index in 1..=count {
        let card = format!("xpath=(//div[@class='quote'])[{index}]");
        let text = page.locator(&format!("{card}//span[@class='text']")).text_content().await?;
        let author = page.locator(&format!("{card}//small[@class='author']")).text_content().await?;
        let tags = page.locator(&format!("{card}//a[@class='tag']")).all_text_contents().await?;
        ctx.push(quotes, &Quote { text, author, tags }).await?;
    }
    let next = page.locator("li.next a");
    ctx.enqueue_links(&next, ctx.request().label(), ()).await?;
    Ok(())
}

/// The handler of the scrolling page, which only the browser serves.
async fn handle_scroll(ctx: CrawlContext<()>, quotes: &Dataset<Quote>) -> Result<(), Error> {
    with_resource_blocking(ctx.page(), extract_scroll(&ctx, quotes)).await
}

/// Reloads the page so its first batch of quotes is observed, then scrolls for each next batch
/// until the API says there is none. Each batch is read from the response the page loaded.
async fn extract_scroll(ctx: &CrawlContext<()>, quotes: &Dataset<Quote>) -> Result<(), Error> {
    let page = ctx.page();
    let (mut response, _) = page
        .wait_for_response(QUOTES_API)
        .timeout(API_WAIT)
        .during(page.reload())
        .await?;
    loop {
        let body = response.json().await?;
        let Some(batch) = api_quotes(&body) else {
            return Err(ctx.retry("a quotes response had an unexpected shape"));
        };
        for quote in &batch {
            ctx.push(quotes, quote).await?;
        }
        match body.get("has_next").and_then(Value::as_bool) {
            Some(true) => {}
            Some(false) => return Ok(()),
            None => return Err(ctx.retry("a quotes response had no has_next flag")),
        }
        let (next, _) = page
            .wait_for_response(QUOTES_API)
            .timeout(API_WAIT)
            .during(page.evaluate::<bool>(SCROLL_TO_END))
            .await?;
        response = next;
    }
}

/// The quotes of one JSON batch, or `None` when the batch does not have the expected shape.
fn api_quotes(body: &Value) -> Option<Vec<Quote>> {
    body.get("quotes")?.as_array()?.iter().map(api_quote).collect::<Option<Vec<Quote>>>()
}

/// One quote of a JSON batch. Its author is either a name or an object with a name.
fn api_quote(item: &Value) -> Option<Quote> {
    let text = item.get("text")?.as_str()?.to_owned();
    let author = match item.get("author")? {
        Value::String(name) => name.clone(),
        Value::Object(author) => author.get("name")?.as_str()?.to_owned(),
        _ => return None,
    };
    let tags = item
        .get("tags")?
        .as_array()?
        .iter()
        .map(|tag| tag.as_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()?;
    Some(Quote { text, author, tags })
}

/// Runs `work` while the page's routes are answered: images and fonts are aborted, and every
/// other request continues. The routes are answered only while `work` is pending.
async fn with_resource_blocking<T>(page: &Page, work: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
    let mut routes = page.route("**/*");
    let blocking = async {
        while let Some(item) = routes.next().await {
            let route = item?;
            if matches!(route.request().resource_type(), ResourceType::Image | ResourceType::Font) {
                route.abort().reason(AbortReason::BlockedByClient).await?;
            } else {
                route.r#continue().await?;
            }
        }
        Ok::<(), Error>(())
    };
    let blocking = pin!(blocking);
    let work = pin!(work);
    match select(blocking, work).await {
        Either::Left((Err(error), _)) => Err(error),
        Either::Left((Ok(()), rest)) => rest.await,
        Either::Right((result, _)) => result,
    }
}
