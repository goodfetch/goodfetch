//! Golden scenario 06, "Crawl with HTTP and a browser only where needed" (Foxhound, Rust).

use goodfetch_foxhound::schemars::JsonSchema;
use goodfetch_foxhound::settings::fetch;
use goodfetch_foxhound::{
    AbortReason, ConnectOptions, Crawl, CrawlContext, ElementState, Error, FetchTier, Foxhound,
    HookOn, LabelDeclaration, PageStageContext, ResourceType, Seed, Settings, UrlMatcher,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A quote, with the version of the site it came from.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Quote {
    text: String,
    author: String,
    tags: Vec<String>,
    version: String,
}

/// A quote's author in the scrolling version's JSON API.
#[derive(Debug, Deserialize)]
struct Author {
    name: String,
}

/// A quote in the scrolling version's JSON API.
#[derive(Debug, Deserialize)]
struct ApiQuote {
    text: String,
    author: Author,
    tags: Vec<String>,
}

/// One page of the scrolling version's JSON API.
#[derive(Debug, Deserialize)]
struct QuotesPage {
    has_next: bool,
    quotes: Vec<ApiQuote>,
}

/// The JSON page `n` the scrolling version loads.
fn api_page(n: u32) -> UrlMatcher {
    UrlMatcher::Regex(format!(r"/api/quotes\?page={n}$"))
}

/// Every quote of the three versions, once per version.
async fn collect_quotes() -> Result<Vec<Quote>, Error> {
    let mut crawl = Crawl::new();
    let quotes = crawl.dataset::<Quote>("quotes")?;
    // The tier is learned per Label on a domain (`fetch.tier: auto`, the default), so each version
    // that may need another tier is its own Label; the two share one handler.
    let html = crawl.label::<()>("html")?;
    let js = crawl.label::<()>("js")?;
    // Scrolling and reading the JSON the page loads need a browser: this Label forces it.
    let browser = Settings::new().set(fetch::TIER, fetch::TierChoice::Forced(FetchTier::Browser));
    let scroll = crawl.label(LabelDeclaration::<()>::new("scroll").settings(browser))?;

    // Runs only where a browser navigates, so unscoped it covers every browser page, a sample
    // comparison of an HTTP Label's page included. The route must outlive the Hook, so the program
    // answers it on a task of its own, which ends when the page closes.
    let no_media = HookOn::every_label().named("no-images-or-fonts");
    crawl.before_navigation(no_media, async |ctx: PageStageContext<Value>| {
        let mut routes = ctx.page().route("**/*");
        tokio::spawn(async move {
            while let Some(route) = routes.next().await {
                let route = route?;
                match route.request().resource_type() {
                    ResourceType::Image | ResourceType::Font => {
                        route.abort().reason(AbortReason::BlockedByClient).await?
                    }
                    _ => route.fallback().await?,
                }
            }
            Ok::<(), Error>(())
        });
        Ok(())
    })?;

    let read_quotes = async |ctx: CrawlContext<()>| {
        let page = ctx.page();
        // What the page must hold: over HTTP, /js/'s document has no quote, so this wait escalates
        // the request to the browser; it never ends as an empty success.
        page.locator(".quote")
            .first()
            .wait_for(ElementState::Attached)
            .await?;
        for quote in page.locator(".quote").all().await? {
            let record = Quote {
                text: quote.locator(".text").text_content().await?,
                author: quote.locator(".author").text_content().await?,
                tags: quote.locator(".tag").all_text_contents().await?,
                version: ctx.request().label().name().to_owned(),
            };
            ctx.push(&quotes, &record).await?;
        }
        // The Frontier fetches each URL once, so each page's quotes are pushed once.
        ctx.enqueue_links(&page.locator("li.next a"), ctx.request().label(), ())
            .await?;
        Ok(())
    };
    crawl.handle(&html, read_quotes)?;
    crawl.handle(&js, read_quotes)?;

    crawl.handle(&scroll, async |ctx: CrawlContext<()>| {
        let page = ctx.page();
        // Page 1 loaded while the engine navigated, before any handler could wait for it: reload
        // with the waiter armed.
        let (mut loaded, _) = page
            .wait_for_response(api_page(1))
            .during(page.reload())
            .await?;
        for n in 2.. {
            // A body that is not JSON fails with `response_not_json`.
            let body: QuotesPage = serde_json::from_value(loaded.json().await?)
                .map_err(|error| ctx.skip(&format!("unexpected JSON: {error}")))?;
            for quote in body.quotes {
                let record = Quote {
                    text: quote.text,
                    author: quote.author.name,
                    tags: quote.tags,
                    version: "scroll".to_owned(),
                };
                ctx.push(&quotes, &record).await?;
            }
            if !body.has_next {
                break;
            }
            // Each JSON page is awaited once, by its number; one that never loads fails the
            // request with `deadline_exceeded`, retried, then `failed`.
            let wheel = page.mouse().wheel(0.0, 10_000.0);
            (loaded, _) = page.wait_for_response(api_page(n)).during(wheel).await?;
        }
        Ok(())
    })?;

    // No browser on the machine: the pinned one is fetched on first use.
    let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    let seeds = [
        Seed::new("https://quotes.toscrape.com/", &html, ()),
        Seed::new("https://quotes.toscrape.com/js/", &js, ()),
        Seed::new("https://quotes.toscrape.com/scroll", &scroll, ()),
    ];
    let run = foxhound.start_crawl(&crawl, seeds).await?;
    // Serving is what lets a crawl finish: it returns the Run's report once the Run ends (ADR 0102).
    let report = crawl.serve(&run).await?;
    let totals = &report.requests;
    println!(
        "{} pages succeeded, {} failed: {:?}",
        totals.succeeded, totals.failed, totals.by_error_code
    );
    let mut collected = Vec::new();
    let mut records = run.records(&quotes);
    while let Some(record) = records.next().await {
        collected.push(record?.value);
    }
    foxhound.close().await?;
    Ok(collected)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    for quote in collect_quotes().await? {
        println!("[{}] {} ({})", quote.version, quote.text, quote.author);
    }
    Ok(())
}
