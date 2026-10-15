//! Golden scenario 08: extract with AI inside a crawl, learning once per page type.
//!
//! The declaration needs no connection, so scenario 10's offline tests include this file and call
//! [`crawl`].

use std::time::Duration;

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::{ai, timeout};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRequestOutcomeKind, Dataset, DatasetDeclaration,
    Error, Foxhound, Label, LabelDeclaration, Money, OutcomeKind, OutcomesFilter, Schema, Seed,
    Settings,
};
use serde::{Deserialize, Serialize};

/// The Plan file: one entry for the book's extraction, compiled once per page layout (ADR 0091).
pub const PLAN: &str = "books.plan.json";

/// What the model extracts from a book's page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BookFields {
    /// The title.
    #[schemars(length(min = 1))]
    pub title: String,
    /// Pounds, exact.
    #[schemars(regex(pattern = r"^\d+\.\d{2}$"))]
    pub price: String,
    /// The number in stock.
    pub in_stock: u32,
    /// The star rating.
    #[schemars(range(min = 1, max = 5))]
    pub rating: u8,
}

/// A book's Record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Book {
    /// What the model extracted.
    #[serde(flatten)]
    pub fields: BookFields,
    /// The book page's final URL.
    pub url: String,
}

/// The crawl's declaration, with the handles a test needs.
#[derive(Debug)]
pub struct BookCrawl {
    /// The Labels and their handlers.
    pub crawl: Crawl<'static>,
    /// The books extracted.
    pub books: Dataset<Book>,
    /// The site's home page, the seed.
    pub home: Label<()>,
    /// A category's pages.
    pub category: Label<()>,
    /// A book's page.
    pub book: Label<()>,
}

/// Declares the crawl. Each handler owns clones of the handles it uses.
pub fn crawl() -> Result<BookCrawl, Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;
    let home = crawl.label::<()>("home")?;
    let category = crawl.label::<()>("category")?;
    // A first compile runs inside its handler, so it gets more than `timeout.handler`'s 60 s.
    let handler_timeout = Settings::new().set(timeout::HANDLER, Duration::from_secs(180));
    let book = crawl.label(LabelDeclaration::<()>::new("book").settings(handler_timeout))?;

    let to_category = category.clone();
    crawl.handle(&home, async move |ctx: CrawlContext<()>| {
        let links = ctx.page().locator(".side_categories ul ul a");
        ctx.enqueue_links(&links, &to_category, ()).limit(3).await?;
        Ok(())
    })?;

    let to_book = book.clone();
    crawl.handle(&category, async move |ctx: CrawlContext<()>| {
        let links = ctx.page().locator("article.product_pod h3 a");
        ctx.enqueue_links(&links, &to_book, ()).await?;
        let next = ctx.page().locator("li.next a");
        ctx.enqueue_links(&next, ctx.request().label(), ()).await?;
        Ok(())
    })?;

    let to_books = books.clone();
    crawl.handle(&book, async move |ctx: CrawlContext<()>| {
        // The first book compiles the Plan entry; the next replay it with no model call, and a
        // page whose layout drifted gets one Repair. Past the Model allowance, a step that needs
        // the model fails with `model_allowance_reached`: returned, it ends this request
        // `skipped` (`model_allowance`).
        let step = ctx
            .page()
            .extract("the book on this page", Schema::<BookFields>::of())
            .await?;
        let record = Book {
            fields: step.output,
            url: ctx.page().url(),
        };
        ctx.push(&to_books, &record).await
    })?;

    Ok(BookCrawl {
        crawl,
        books,
        home,
        category,
        book,
    })
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let BookCrawl {
        crawl, home, book, ..
    } = crawl()?;
    let model = Settings::new().set(ai::MODEL, Some("vercel/openai/gpt-5-mini".into()));
    let foxhound = Foxhound::connect(ConnectOptions::default().settings(model)).await?;
    // The Model allowance: not a Limit, so the crawl goes on once it is spent.
    let allowance = Settings::new().set(ai::allowance::PRICE, Some(Money::new("0.50", "USD")?));
    let run = foxhound
        .start_crawl(
            &crawl,
            [Seed::new("https://books.toscrape.com/", &home, ())],
        )
        .plan(PLAN)
        .settings(allowance)
        .await?;
    // Serving is what lets a crawl finish: it returns the Run's report once the Run ends (ADR
    // 0102), `incomplete` when a book was skipped for the allowance.
    let report = crawl.serve(&run).await?;

    let left = OutcomesFilter::default().kinds(&[OutcomeKind::Failed, OutcomeKind::Skipped]);
    let mut left = run.outcomes(left.label(&book));
    while let Some(outcome) = left.next().await {
        let outcome = outcome?;
        let url = &outcome.request.url;
        match &outcome.kind {
            CrawlRequestOutcomeKind::Skipped { reason, message } => {
                println!(
                    "unextracted {url}: {reason:?} {}",
                    message.as_deref().unwrap_or("")
                );
            }
            CrawlRequestOutcomeKind::Failed(error) => {
                let fix = error.fix().unwrap_or("");
                println!("unextracted {url}: {} {fix}", error.code().as_str());
            }
            _ => {}
        }
    }

    let usage = &report.run.usage.model;
    let price = usage
        .price
        .as_ref()
        .map(|p| format!("{} {}", p.amount(), p.currency()));
    println!(
        "model use: {} tokens, {}",
        usage.tokens,
        price.as_deref().unwrap_or("unpriced")
    );
    run.as_run().close().await?;
    foxhound.close().await
}
