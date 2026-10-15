//! Crawls books.toscrape.com: home page, then each category (with its pagination), then each book.
//! Every book becomes one Record of the `books` Dataset, exported to `books.json` when the crawl
//! ends; the program then reports the outcome of every request.

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::crawl::scope::{STRATEGY, Strategy};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRequestOutcomeKind, DatasetDeclaration, Error,
    ExportFormat, Foxhound, LinkInfo, OutcomeKind, OutcomesFilter, Seed, Settings, error_code,
};
use serde::{Deserialize, Serialize};

const START_URL: &str = "https://books.toscrape.com/";
const EXPORT_PATH: &str = "books.json";

/// What a category page passes on to its book pages (and to its next page).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct CategoryName {
    category: String,
}

/// One book, as a Record of the `books` Dataset.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Book {
    title: String,
    /// Pounds.
    price: f64,
    in_stock: u32,
    /// One to five.
    rating: u8,
    url: String,
    category: String,
}

/// A page whose content did not have the expected shape fails its request.
fn malformed(what: &str, url: &str, text: &str) -> Error {
    Error::fake(
        error_code::HANDLER_FAILED,
        format!("cannot read the {what} from {url}: {text:?}"),
    )
}

fn parse_price(text: &str) -> Option<f64> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    digits.parse().ok()
}

fn parse_stock(text: &str) -> Option<u32> {
    text.split(|c: char| !c.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse().ok())
}

fn parse_rating(class: &str) -> Option<u8> {
    class.split_whitespace().find_map(|word| match word {
        "One" => Some(1),
        "Two" => Some(2),
        "Three" => Some(3),
        "Four" => Some(4),
        "Five" => Some(5),
        _ => None,
    })
}

async fn crawl_site(fox: &Foxhound) -> Result<(), Error> {
    let mut crawl = Crawl::new();

    let home = crawl.label::<()>("home")?;
    let category = crawl.label::<CategoryName>("category")?;
    let book = crawl.label::<CategoryName>("book")?;
    let books = crawl.dataset::<Book>(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;

    // Home page: every category link of the side list but the top-level "Books" one, which is the
    // outer list's own entry; the category's name travels as the Payload.
    crawl.handle(&home, async |ctx: CrawlContext<()>| -> Result<(), Error> {
        let links = ctx.page().locator("div.side_categories ul ul a");
        ctx.enqueue_links_with(&links, &category, |link: &LinkInfo| {
            Some(CategoryName { category: link.text.trim().to_owned() })
        })
        .await?;
        Ok(())
    })?;

    // Category page: its books, then its next page, which keeps the category's name.
    crawl.handle(&category, async |ctx: CrawlContext<CategoryName>| -> Result<(), Error> {
        let name = ctx.request().payload().clone();
        let book_links = ctx.page().locator("article.product_pod h3 a");
        ctx.enqueue_links(&book_links, &book, name.clone()).await?;
        let next_link = ctx.page().locator("li.next a");
        ctx.enqueue_links(&next_link, ctx.request().label(), name).await?;
        Ok(())
    })?;

    // Book page: one Record.
    crawl.handle(&book, async |ctx: CrawlContext<CategoryName>| -> Result<(), Error> {
        let page = ctx.page();
        let url = page.url();
        let title = page.locator("div.product_main h1").text_content().await?;
        let price_text = page.locator("div.product_main p.price_color").text_content().await?;
        let stock_text = page.locator("div.product_main p.availability").text_content().await?;
        let rating_class = page
            .locator("div.product_main p.star-rating")
            .get_attribute("class")
            .await?
            .unwrap_or_default();

        let price = parse_price(&price_text).ok_or_else(|| malformed("price", &url, &price_text))?;
        let in_stock =
            parse_stock(&stock_text).ok_or_else(|| malformed("stock", &url, &stock_text))?;
        let rating =
            parse_rating(&rating_class).ok_or_else(|| malformed("rating", &url, &rating_class))?;

        let record = Book {
            title: title.trim().to_owned(),
            price,
            in_stock,
            rating,
            url,
            category: ctx.request().payload().category.clone(),
        };
        ctx.push(&books, &record).await?;
        Ok(())
    })?;

    // Seeds define the scope; the strategy is set out loud so the crawl stays on the site.
    let settings = Settings::new().set(STRATEGY, Strategy::SameHostname);
    let run = fox
        .start_crawl(&crawl, [Seed::new(START_URL, &home, ())])
        .settings(settings)
        .await?;

    let report = crawl.serve(&run).await?;

    let export = run.export(&books, ExportFormat::Json).await?;
    export.save_as(EXPORT_PATH).await?;
    println!("exported {} books to {EXPORT_PATH}", export.records());

    let requests = &report.requests;
    println!(
        "pages: {} succeeded, {} failed, {} skipped, {} blocked, {} left pending",
        requests.succeeded, requests.failed, requests.skipped, requests.blocked, requests.pending
    );
    for (reason, count) in &requests.by_skip_reason {
        println!("  skipped, {count} x {reason:?}");
    }
    for (code, count) in &requests.by_error_code {
        println!("  failed, {count} x {}", code.as_str());
    }
    for (cause, count) in &requests.by_block_cause {
        println!("  blocked, {count} x {cause:?}");
    }

    let mut outcomes = run.outcomes(
        OutcomesFilter::default().kinds(&[OutcomeKind::Failed, OutcomeKind::Skipped, OutcomeKind::Blocked]),
    );
    while let Some(outcome) = outcomes.next().await {
        let outcome = outcome?;
        let url = &outcome.request.url;
        match &outcome.kind {
            CrawlRequestOutcomeKind::Failed(error) => println!(
                "failed {url} after {} attempts: {} ({})",
                outcome.attempts,
                error.code().as_str(),
                error.message()
            ),
            CrawlRequestOutcomeKind::Skipped { reason, message } => {
                println!("skipped {url}: {reason:?} {}", message.as_deref().unwrap_or(""));
            }
            CrawlRequestOutcomeKind::Blocked(cause) => println!("blocked {url}: {cause:?}"),
            _ => {}
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let fox = Foxhound::connect(ConnectOptions::default()).await?;
    let result = crawl_site(&fox).await;
    fox.close().await?;
    result
}
