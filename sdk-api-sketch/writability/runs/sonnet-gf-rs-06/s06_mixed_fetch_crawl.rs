//! Collects every quote of the three versions of quotes.toscrape.com with one crawl.
//!
//! One Foxhound crawl, one connection. The plain HTML and the JavaScript-rendered versions share
//! one `quotes` Label and one handler: the handler waits for the quotes to be in the document, so
//! the plain pages are served over HTTP and the rendered ones, whose HTTP document lacks quotes,
//! are run again in the browser by the engine. A page whose quotes are missing is never a success.
//! The scrolling version has its own Label, forced onto the browser, and reads the JSON responses
//! the page loads. A before-navigation Hook, which only browser attempts run, keeps images and
//! fonts from being downloaded.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::poll_fn;
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;

use futures_util::future::{Either, FutureExt, LocalBoxFuture, select};
use futures_util::stream::{FuturesUnordered, StreamExt};
use goodfetch_foxhound::settings::fetch::TierChoice;
use goodfetch_foxhound::settings::{self, Settings};
use goodfetch_foxhound::{
    AbortReason, ConnectOptions, Count, Crawl, CrawlContext, Dataset, DatasetDeclaration, Error,
    EventStream, FetchTier, Foxhound, HookOn, LabelDeclaration, PageStageContext, ResourceType,
    Route, Seed, UrlMatcher,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which version of the site a quote was collected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Version {
    /// `https://quotes.toscrape.com/`.
    Html,
    /// `https://quotes.toscrape.com/js/`.
    Js,
    /// `https://quotes.toscrape.com/scroll`.
    Scroll,
}

/// One Record: a quote of one version.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Quote {
    version: Version,
    text: String,
    author: String,
    tags: Vec<String>,
}

/// One page of the scrolling version's JSON API.
#[derive(Debug, Deserialize)]
struct ApiPage {
    quotes: Vec<ApiQuote>,
    has_next: bool,
}

#[derive(Debug, Deserialize)]
struct ApiQuote {
    text: String,
    author: ApiAuthor,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ApiAuthor {
    name: String,
}

fn clean(text: &str) -> String {
    text.trim().trim_matches(['\u{201c}', '\u{201d}']).trim().to_owned()
}

/// The handler of both the plain and the rendered listing pages, on whatever tier serves them.
async fn handle_listing(quotes: &Dataset<Quote>, ctx: CrawlContext<Version>) -> Result<(), Error> {
    let version = *ctx.request().payload();
    let page = ctx.page();

    // Over HTTP a document without quotes cannot satisfy this wait: the engine runs the request
    // again in the browser, where the script has rendered them.
    let rows = page.locator(".quote");
    let found = rows.wait_for_count(Count::at_least(1)).await?;
    if found == 0 {
        return Err(ctx.retry("no quotes on the page"));
    }

    for row in rows.all().await? {
        let text = row.locator(".text").text_content().await?;
        let author = row.locator(".author").text_content().await?;
        let tags = row.locator(".tags .tag").all_text_contents().await?;
        let quote = Quote {
            version,
            text: clean(&text),
            author: author.trim().to_owned(),
            tags: tags.iter().map(|tag| tag.trim().to_owned()).collect(),
        };
        ctx.push(quotes, &quote).await?;
    }

    // Pagination stays in the same Label, with the same version.
    ctx.enqueue_links(&page.locator("li.next a"), ctx.request().label(), version).await?;
    Ok(())
}

/// The matcher of the scrolling version's API response for page `number`.
fn api_page(number: u32) -> UrlMatcher {
    UrlMatcher::Regex(format!(r"/api/quotes\?page={number}(&|$)"))
}

/// The scrolling version: its quotes come from the JSON responses the page loads.
async fn handle_scroll(quotes: &Dataset<Quote>, ctx: CrawlContext<()>) -> Result<(), Error> {
    let page = ctx.page();
    let mut number = 1;
    let mut pushed = 0_usize;

    // The first response was loaded before the handler ran: load the page again to read it.
    let (mut response, _) = page.wait_for_response(api_page(number)).during(page.reload()).await?;
    loop {
        let body: Value = response.json().await?;
        let api: ApiPage = serde_json::from_value(body)
            .map_err(|error| ctx.retry(&format!("unexpected quotes JSON: {error}")))?;
        for quote in api.quotes {
            let record = Quote {
                version: Version::Scroll,
                text: clean(&quote.text),
                author: quote.author.name,
                tags: quote.tags,
            };
            ctx.push(quotes, &record).await?;
            pushed += 1;
        }
        if !api.has_next {
            break;
        }
        // Scrolling makes the page ask for the next page of quotes.
        number += 1;
        (response, _) = page
            .wait_for_response(api_page(number))
            .during(page.mouse().wheel(0.0, 100_000.0))
            .await?;
    }

    if pushed == 0 {
        return Err(ctx.retry("the JSON responses held no quotes"));
    }
    Ok(())
}

/// Answers a page's requests: images and fonts are aborted, everything else goes on.
async fn block_images_and_fonts(mut routes: EventStream<Route>) {
    while let Some(item) = routes.next().await {
        let Ok(route) = item else { break };
        let heavy = matches!(route.request().resource_type(), ResourceType::Image | ResourceType::Font);
        let answered = if heavy {
            route.abort().reason(AbortReason::BlockedByClient).await
        } else {
            route.r#continue().await
        };
        if answered.is_err() {
            break;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let fox = Foxhound::connect(ConnectOptions::default()).await?;

    // Route streams opened by the Hook below; the async API spawns nothing, so the program drives
    // them beside `serve`.
    let routes: RefCell<FuturesUnordered<LocalBoxFuture<'static, ()>>> =
        RefCell::new(FuturesUnordered::new());

    let mut crawl = Crawl::new();
    let quotes = crawl.dataset(DatasetDeclaration::<Quote>::new("quotes").key(&["version", "text"]))?;
    let listing = crawl.label::<Version>("listing")?;
    let scroll = crawl.label::<()>(
        LabelDeclaration::<()>::new("scroll").settings(
            Settings::new().set(settings::fetch::TIER, TierChoice::Forced(FetchTier::Browser)),
        ),
    )?;

    let listing_quotes = quotes.clone();
    crawl.handle(&listing, async move |ctx: CrawlContext<Version>| {
        handle_listing(&listing_quotes, ctx).await
    })?;
    let scroll_quotes = quotes.clone();
    crawl.handle(&scroll, async move |ctx: CrawlContext<()>| handle_scroll(&scroll_quotes, ctx).await)?;

    // Only browser attempts reach this stage, so pages fetched over HTTP are not affected.
    crawl.before_navigation(HookOn::every_label(), async |ctx: PageStageContext<Value>| {
        routes.borrow().push(block_images_and_fonts(ctx.page().route("**/*")).boxed_local());
        Ok(())
    })?;

    let seeds = [
        Seed::new("https://quotes.toscrape.com/", &listing, Version::Html),
        Seed::new("https://quotes.toscrape.com/js/", &listing, Version::Js),
        Seed::new("https://quotes.toscrape.com/scroll", &scroll, ()),
    ];
    let run = fox
        .start_crawl(&crawl, seeds)
        .settings(Settings::new().set(settings::timeout::HANDLER, Duration::from_secs(180)))
        .await?;

    let driver = poll_fn(|cx| {
        let mut routes = routes.borrow_mut();
        while let Poll::Ready(Some(())) = routes.poll_next_unpin(cx) {}
        Poll::<Infallible>::Pending
    });
    let report = match select(pin!(crawl.serve(&run)), pin!(driver)).await {
        Either::Left((report, _)) => report?,
        Either::Right((never, _)) => match never {},
    };

    let mut per_version: BTreeMap<Version, usize> = BTreeMap::new();
    let mut records = run.records(&quotes);
    while let Some(record) = records.next().await {
        *per_version.entry(record?.value.version).or_default() += 1;
    }
    println!("requests: {:?}", report.requests);
    println!("completeness: {:?}", report.run.completeness);
    for (version, count) in &per_version {
        println!("{version:?}: {count} quotes");
    }

    fox.close().await?;
    Ok(())
}
