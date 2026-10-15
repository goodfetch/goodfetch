//! Crawls books.toscrape.com with Foxhound: the home page's category links (not the top-level
//! "Books" link), each category page's book links and its "next" link, and one Record per book.
//! The Records go to the `books` Dataset, which is exported to `books.json` when the crawl ends.

use goodfetch_foxhound::{
    Crawl, CrawlRequestOutcomeKind, ConnectOptions, Error, ExportFormat, Foxhound, LinkInfo,
    OutcomesFilter, Seed,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The site's home page, where the crawl starts.
const HOME: &str = "https://books.toscrape.com/";
/// The file the `books` Dataset is exported to.
const EXPORT_FILE: &str = "books.json";

/// The payload of a category page and of a book page: the category the book was reached from.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct CategoryPayload {
    /// The category's name, as the side list shows it.
    category: String,
}

/// One book, one Record of the `books` Dataset.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Book {
    /// The book's title.
    title: String,
    /// The price in pounds.
    price: f64,
    /// The number of copies in stock.
    stock: u32,
    /// The star rating, from one to five.
    #[schemars(range(min = 1, max = 5))]
    rating: u8,
    /// The address of the book's page.
    url: String,
    /// The category the book was reached from.
    category: String,
}

/// The price in pounds from text such as "£51.77".
fn parse_price(text: &str) -> Option<f64> {
    text.trim().strip_prefix('£')?.parse().ok()
}

/// The number in stock from text such as "In stock (22 available)".
fn parse_stock(text: &str) -> Option<u32> {
    let after = text.split('(').nth(1)?;
    after.split_whitespace().next()?.parse().ok()
}

/// The star rating from a class attribute such as "star-rating Three".
fn parse_rating(class: &str) -> Option<u8> {
    match class.split_whitespace().last()? {
        "One" => Some(1),
        "Two" => Some(2),
        "Three" => Some(3),
        "Four" => Some(4),
        "Five" => Some(5),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut crawl = Crawl::new();

    let home_label = crawl.label::<()>("home")?;
    let category_label = crawl.label::<CategoryPayload>("category")?;
    let book_label = crawl.label::<CategoryPayload>("book")?;
    let books = crawl.dataset::<Book>("books")?;

    // The home page: every category link in the side list, except the top-level "Books" one,
    // which is not nested in a list of its own.
    crawl.handle(&home_label, async |ctx| {
        let side_list = ctx.page().locator(".side_categories ul ul a");
        ctx.enqueue_links_with(&side_list, &category_label, |link: &LinkInfo| {
            Some(CategoryPayload {
                category: link.text.trim().to_owned(),
            })
        })
        .await?;
        Ok(())
    })?;

    // A category page: its book links, and its "next" page link, which carries the same category.
    crawl.handle(&category_label, async |ctx| {
        let category = ctx.request().payload().category.clone();
        let page = ctx.page();
        let book_links = page.locator("article.product_pod h3 a");
        let next_link = page.locator("li.next a");
        ctx.enqueue_links_with(&book_links, &book_label, |_link: &LinkInfo| {
            Some(CategoryPayload {
                category: category.clone(),
            })
        })
        .await?;
        ctx.enqueue_links(
            &next_link,
            &category_label,
            CategoryPayload {
                category: category.clone(),
            },
        )
        .await?;
        Ok(())
    })?;

    // A book page: one Record. A page whose values do not read as the book's is skipped with the
    // reason; a missing element fails the attempt with the engine's own error.
    crawl.handle(&book_label, async |ctx| {
        let category = ctx.request().payload().category.clone();
        let url = ctx.page().url();
        let page = ctx.page();

        let title = page.locator(".product_main h1").text_content().await?;
        let price_text = page.locator(".product_main .price_color").text_content().await?;
        let stock_text = page.locator(".product_main .availability").text_content().await?;
        let rating_class = page
            .locator(".product_main .star-rating")
            .get_attribute("class")
            .await?
            .unwrap_or_default();

        let Some(price) = parse_price(&price_text) else {
            return Err(ctx.skip(&format!("price {:?} is not a pound amount", price_text.trim())));
        };
        let Some(stock) = parse_stock(&stock_text) else {
            return Err(ctx.skip(&format!("stock {:?} gives no count", stock_text.trim())));
        };
        let Some(rating) = parse_rating(&rating_class) else {
            return Err(ctx.skip(&format!("rating class {rating_class:?} is not one to five")));
        };

        let book = Book {
            title: title.trim().to_owned(),
            price,
            stock,
            rating,
            url,
            category,
        };
        ctx.push(&books, &book).await?;
        Ok(())
    })?;

    let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    let run = foxhound
        .start_crawl(&crawl, [Seed::new(HOME, &home_label, ())])
        .await?;

    let report = crawl.serve(&run).await?;

    let export = run.export(&books, ExportFormat::Json).await?;
    export.save_as(EXPORT_FILE).await?;
    println!("exported {} books to {EXPORT_FILE}", export.records());

    let totals = &report.requests;
    println!("pages succeeded: {}", totals.succeeded);
    println!("pages failed: {}", totals.failed);
    println!("pages skipped: {}", totals.skipped);
    println!("pages blocked: {}", totals.blocked);
    println!("pages pending: {}", totals.pending);

    let mut outcomes = run.outcomes(OutcomesFilter::default());
    while let Some(item) = outcomes.next().await {
        let outcome = item?;
        let url = &outcome.request.url;
        match &outcome.kind {
            CrawlRequestOutcomeKind::Failed(error) => {
                println!("failed {url}: {} ({})", error.code().as_str(), error.message());
            }
            CrawlRequestOutcomeKind::Skipped { reason, message } => {
                println!(
                    "skipped {url}: {reason:?} ({})",
                    message.as_deref().unwrap_or("no message")
                );
            }
            CrawlRequestOutcomeKind::Blocked(cause) => {
                println!("blocked {url}: {cause:?}");
            }
            _ => {}
        }
    }

    foxhound.close().await?;
    Ok(())
}
