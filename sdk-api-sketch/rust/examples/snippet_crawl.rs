//! Snippet for Crawl definition and handler context (#477, ADR 0093), on golden scenarios 05, 06
//! and 08.

use std::time::Duration;

use goodfetch_sdk_sketch::schemars::{self, JsonSchema};
use goodfetch_sdk_sketch::settings::{crawl, fetch, timeout};
use goodfetch_sdk_sketch::{
    ConnectOptions, Crawl, CrawlContext, ElementState, Enqueued, Error, FetchTier, Goodfetch, LabelDeclaration, Schema, Seed, Settings,
};
use serde::{Deserialize, Serialize};

// `page.extract` is ADR 0091's; Datasets and `push` are ADR 0103's (snippet_output.rs).

/// Scenario 05: a book, the category passed from its category page.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Book {
    title: String,
    /// Pounds, two decimals, kept as text.
    price: String,
    in_stock: u32,
    url: String,
    category: String,
}

/// The Payload of a category request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct CategoryPayload {
    name: String,
}

/// The Payload of a book request.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookPayload {
    category: String,
}

/// Scenario 05: three kinds of page. Handlers borrow `books` and the Labels: they run inside the
/// future of `serve`, so they need neither `Send` nor `'static` (ADR 0081).
async fn scenario_05() -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset_with("books", Schema::<Book>::of(), &["url"])?;
    let home = crawl.label::<()>("home")?;
    let category = crawl.label::<CategoryPayload>("category")?;
    let book = crawl.label(
        LabelDeclaration::<BookPayload>::new("book").settings(Settings::new().set(timeout::HANDLER, Duration::from_secs(30))),
    )?;

    crawl.handle(&home, async |ctx: CrawlContext<()>| {
        for link in ctx.page().locator(".side_categories ul ul a").all().await? {
            if let Some(href) = link.get_attribute("href").await? {
                let name = link.text_content().await?.trim().to_owned();
                ctx.enqueue(&href, &category, CategoryPayload { name }).await?;
            }
        }
        Ok(())
    })?;

    crawl.handle(&category, async |ctx: CrawlContext<CategoryPayload>| {
        let name = ctx.request().payload().name.clone();
        let links = ctx.page().locator("article.product_pod h3 a");
        let added = ctx.enqueue_links(&links, &book, BookPayload { category: name.clone() }).await?;
        if added.added.is_empty() {
            tracing_free_note(&added.not_added);
        }
        // Pagination: the request's own Label.
        let next = ctx.page().locator("li.next a");
        ctx.enqueue_links(&next, ctx.request().label(), CategoryPayload { name }).await?;
        Ok(())
    })?;

    crawl.handle(&book, async |ctx: CrawlContext<BookPayload>| {
        let main = ctx.page().locator(".product_main");
        // Waits; over HTTP, a missing price escalates the request to the browser.
        let price = main.locator(".price_color").text_content().await?;
        let Some(pounds) = price.strip_prefix('£') else {
            return Err(ctx.skip(&format!("no price in {price:?}"))); // `skipped`, reason `handler`
        };
        // Staged with the attempt, committed with it (ADR 0103).
        ctx.push(
            &books,
            &Book {
                title: main.locator("h1").text_content().await?,
                price: pounds.to_owned(),
                in_stock: 0,
                url: ctx.page().url(), // the final URL, after redirects
                category: ctx.request().payload().category.clone(),
            },
        )
        .await
    })?;

    let gf = Goodfetch::connect(ConnectOptions::default()).await?;
    let run = gf
        .foxhound()
        .start_crawl(&crawl, [Seed::new("https://books.toscrape.com/", &home, ())])
        .settings(Settings::new().set(crawl::scope::STRATEGY, crawl::scope::Strategy::SameHostname))
        .await?;
    let report = crawl.serve(&run).await?; // answers until the Run ends, then its report (ADR 0102)
    let _ = (report.requests.succeeded, report.run.completeness);
    run.as_run().close().await?;
    gf.close().await
}

/// What a program logs when a category page enqueued no book.
fn tracing_free_note(not_added: &[Enqueued]) {
    let _ = not_added;
}

/// Scenario 06: one handler over HTTP, the JavaScript version and the scrolling one; the scrolling
/// page's Label forces the browser, which alone reads the JSON the page loads.
async fn scenario_06(gf: &Goodfetch) -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let quotes = crawl.label::<()>("quotes")?;
    let scroll = crawl.label(
        LabelDeclaration::<()>::new("scroll").settings(Settings::new().set(fetch::TIER, fetch::TierChoice::Forced(FetchTier::Browser))),
    )?;
    crawl.handle(&quotes, async |ctx: CrawlContext<()>| {
        // Over HTTP, /js/'s empty document escalates this wait to the browser (failure point 1).
        ctx.page().locator(".quote").first().wait_for(ElementState::Visible).await?;
        let texts = ctx.page().locator(".quote .text").all_text_contents().await?;
        let _ = (texts, ctx.tier());
        ctx.enqueue_links(&ctx.page().locator("li.next a"), ctx.request().label(), ()).await?;
        Ok(())
    })?;
    crawl.handle(&scroll, async |ctx: CrawlContext<()>| {
        let (last, ()) = ctx
            .page()
            .wait_for_request_finished("**/api/quotes?page=*")
            .during(ctx.page().mouse().wheel(0.0, 10_000.0))
            .await?;
        let _ = last;
        Ok(())
    })?;
    let run = gf
        .foxhound()
        .start_crawl(
            &crawl,
            [
                Seed::new("https://quotes.toscrape.com/", &quotes, ()),
                Seed::new("https://quotes.toscrape.com/js/", &quotes, ()),
                Seed::new("https://quotes.toscrape.com/scroll", &scroll, ()),
            ],
        )
        .await?;
    let _report = crawl.serve(&run).await?;
    Ok(())
}

/// The fields an AI extraction reads, for scenario 08.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookFields {
    title: String,
    price: String,
}

/// Scenario 08: an AI extraction on the crawl's own page, counted in the crawl's Run.
async fn scenario_08(gf: &Goodfetch) -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let book = crawl.label::<()>("book")?;
    crawl.handle(&book, async |ctx: CrawlContext<()>| {
        let fields = ctx.page().extract("the book on this page", Schema::<BookFields>::of()).await?.output;
        if fields.title.is_empty() {
            return Err(ctx.retry("the title had not rendered")); // `retry_requested`
        }
        Ok(())
    })?;
    let run = gf.foxhound().start_crawl(&crawl, [Seed::new("https://books.toscrape.com/", &book, ())]).await?;
    let _report = crawl.serve(&run).await?;
    Ok(())
}

fn main() {
    let _ = (scenario_05, scenario_06, scenario_08);
    let _ = schemars::schema_for!(Book);
}
