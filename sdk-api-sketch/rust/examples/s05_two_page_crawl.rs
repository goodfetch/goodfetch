//! Golden scenario 05: crawl two page types into a Dataset. Scenarios 07 and 10 include this file
//! as a module (`#[path = "s05_two_page_crawl.rs"] mod two_page_crawl;`) and build its crawl with
//! [`books_crawl`].

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRequestOutcomeKind, Dataset, DatasetDeclaration,
    Error, ExportFormat, Foxhound, Label, LinkInfo, OutcomeKind, OutcomesFilter, Seed,
};
use serde::{Deserialize, Serialize};

/// One book, as the `books` Dataset holds it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Book {
    /// The book's title.
    #[schemars(length(min = 1))]
    pub title: String,
    /// Pounds, an exact decimal kept as text.
    #[schemars(regex(pattern = r"^\d+\.\d{2}$"))]
    pub price: String,
    /// The number in stock.
    pub in_stock: u32,
    /// One to five stars.
    #[schemars(range(min = 1, max = 5))]
    pub rating: u8,
    /// The page's final URL.
    #[schemars(url)]
    pub url: String,
    /// The category the book was reached from.
    #[schemars(length(min = 1))]
    pub category: String,
}

/// The Payload of a category request: its name, read on the home page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CategoryPayload {
    /// The category's name.
    #[schemars(length(min = 1))]
    pub name: String,
}

/// The Payload of a book request: the category it was reached from.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BookPayload {
    /// The category's name, passed from the category page.
    #[schemars(length(min = 1))]
    pub category: String,
}

/// Scenario 05's crawl and the handles a program needs to run it and read it.
pub struct BooksCrawl {
    /// The declaration: three Labels, one handler each.
    pub crawl: Crawl<'static>,
    /// The Dataset of books, keyed by URL.
    pub books: Dataset<Book>,
    /// The seed's Label.
    pub home: Label<()>,
    /// A book page's Label, for a test of its handler alone.
    pub book: Label<BookPayload>,
}

impl BooksCrawl {
    /// The crawl's seeds. Staying on books.toscrape.com is the default scope (the seed's host),
    /// and robots.txt, one request at a time with a second between starts and backing off on 429
    /// are the default Politeness.
    pub fn seeds(&self) -> [Seed; 1] {
        [Seed::new("https://books.toscrape.com/", &self.home, ())]
    }
}

const RATINGS: [&str; 5] = ["One", "Two", "Three", "Four", "Five"];

/// The text between `before` and `after`.
fn between<'t>(text: &'t str, before: &str, after: &str) -> Option<&'t str> {
    let (_, rest) = text.split_once(before)?;
    Some(rest.split_once(after)?.0)
}

/// Declares the crawl, with no connection and no I/O. Each handler owns a clone of the handles it
/// uses, so the declaration outlives this function.
pub fn books_crawl() -> Result<BooksCrawl, Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;
    let home = crawl.label::<()>("home")?;
    let category = crawl.label::<CategoryPayload>("category")?;
    let book = crawl.label::<BookPayload>("book")?;

    let to_category = category.clone();
    crawl.handle(&home, async move |ctx: CrawlContext<()>| {
        // The nested list leaves out the top-level "Books"; each link's own text is its Payload.
        let links = ctx.page().locator(".side_categories ul ul a");
        let payload = |link: &LinkInfo| {
            Some(CategoryPayload {
                name: link.text.trim().to_owned(),
            })
        };
        ctx.enqueue_links_with(&links, &to_category, payload)
            .await?;
        Ok(())
    })?;

    let to_book = book.clone();
    crawl.handle(
        &category,
        async move |ctx: CrawlContext<CategoryPayload>| {
            let payload = ctx.request().payload().clone();
            let links = ctx.page().locator("article.product_pod h3 a");
            let to = BookPayload {
                category: payload.name.clone(),
            };
            ctx.enqueue_links(&links, &to_book, to).await?;
            let next = ctx.page().locator("li.next a");
            ctx.enqueue_links(&next, ctx.request().label(), payload)
                .await?;
            Ok(())
        },
    )?;

    let into = books.clone();
    crawl.handle(&book, async move |ctx: CrawlContext<BookPayload>| {
        let main = ctx.page().locator(".product_main");
        let text = main.text_content().await?;
        let price = text.split_once('£').map(|(_, rest)| {
            rest.chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>()
        });
        let in_stock = between(&text, "(", " available)").and_then(|count| count.parse().ok());
        let stars = main
            .locator(".star-rating")
            .get_attribute("class")
            .await?
            .unwrap_or_default();
        let rating = RATINGS.iter().position(|word| stars.contains(word));
        let (Some(price), Some(in_stock), Some(rating)) =
            (price.filter(|p| !p.is_empty()), in_stock, rating)
        else {
            let message = format!("no price, stock or rating on the page (stars {stars:?})");
            return Err(ctx.skip(&message));
        };
        let record = Book {
            title: main.locator("h1").text_content().await?,
            price,
            in_stock,
            rating: rating as u8 + 1,
            url: ctx.page().url(), // the final URL, after redirects
            category: ctx.request().payload().category.clone(),
        };
        ctx.push(&into, &record).await?;
        Ok(())
    })?;

    Ok(BooksCrawl {
        crawl,
        books,
        home,
        book,
    })
}

/// Runs the crawl, exports its Dataset and reports its requests.
pub async fn run(foxhound: &Foxhound) -> Result<(), Error> {
    let declared = books_crawl()?;
    let run = foxhound
        .start_crawl(&declared.crawl, declared.seeds())
        .await?;
    let report = declared.crawl.serve(&run).await?; // answers until the Run ends, then its report
    let exported = run.export(&declared.books, ExportFormat::Json).await?;
    exported.save_as("books.json").await?;
    println!("exported {} books to books.json", exported.records());

    let kinds = [
        OutcomeKind::Failed,
        OutcomeKind::Skipped,
        OutcomeKind::Blocked,
    ];
    let mut outcomes = run.outcomes(OutcomesFilter::default().kinds(&kinds));
    while let Some(outcome) = outcomes.next().await {
        let outcome = outcome?;
        let url = &outcome.request.url;
        match &outcome.kind {
            CrawlRequestOutcomeKind::Failed(error) => {
                println!("failed {url}: {:?}: {}", error.code(), error.message());
            }
            CrawlRequestOutcomeKind::Skipped { reason, message } => {
                println!(
                    "skipped {url}: {reason:?} {}",
                    message.as_deref().unwrap_or("")
                );
            }
            CrawlRequestOutcomeKind::Blocked(cause) => println!("blocked {url}: {cause:?}"),
            _ => {}
        }
    }
    let totals = &report.requests;
    println!(
        "{} succeeded, {} failed, {} skipped, {} blocked",
        totals.succeeded, totals.failed, totals.skipped, totals.blocked
    );
    if let Some(nothing_done) = &report.run.nothing_done {
        println!("nothing done: {nothing_done:?}");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    Foxhound::scope(ConnectOptions::default(), run).await
}
