//! Crawls the first three categories of the side list of https://books.toscrape.com/ and every book
//! they list, following each category's "next" pages. Each book's title, price, stock count and star
//! rating are extracted by Collie's AI `extract`, compiled once per page type: later books of the same
//! layout replay what the model learned with no model call. The Run's model allowance of 0.50 USD
//! stops every model call once spent; the crawl then finishes without the model, and the books it
//! could not extract are listed with their reasons.

use std::cell::RefCell;

use goodfetch_foxhound::settings::{self, Settings, ai};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRequestOutcomeKind, Error, ExtractMode, Foxhound,
    ModelReference, Money, OutcomesFilter, Schema, Seed, StepSource,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The model that reads the pages, through the Vercel AI Gateway's free tier.
const MODEL_REFERENCE: &str = "vercel/openai/gpt-4o-mini";
/// The model allowance of the whole Run, in USD.
const MODEL_ALLOWANCE_USD: &str = "0.50";
/// The first page of the books site, whose side list holds the categories.
const HOME_URL: &str = "https://books.toscrape.com/";
/// The side list's categories, below the "Books" entry that holds them all.
const CATEGORY_LINKS: &str = "div.side_categories ul ul li a";
/// How many categories the crawl takes, in the side list's order.
const CATEGORY_LIMIT: u32 = 3;
/// The book links on a category page.
const BOOK_LINKS: &str = "article.product_pod h3 a";
/// The "next" link of a category page.
const NEXT_PAGE_LINK: &str = "li.next a";
/// The instruction the model compiles once, for every book page of the same layout.
const BOOK_INSTRUCTION: &str = "From this book's page, read its title, its price as shown (such as \
    £51.77), the number of copies in stock from the availability text, and its star rating as a \
    whole number from 1 to 5.";

/// The fields the model extracts from one book page.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct BookFacts {
    /// The book's title.
    title: String,
    /// The price as shown on the page, such as "£51.77".
    price: String,
    /// The number of copies in stock.
    in_stock: u32,
    /// The star rating, from 1 to 5.
    star_rating: u8,
}

/// One book as the crawl commits it to the `books` Dataset.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct BookRecord {
    /// The book's page URL.
    url: String,
    /// The title.
    title: String,
    /// The price as shown.
    price: String,
    /// The number of copies in stock.
    in_stock: u32,
    /// The star rating, from 1 to 5.
    star_rating: u8,
}

/// How the book pages were extracted, counted as their handlers finish.
#[derive(Debug, Default)]
struct Tally {
    /// Books extracted by a model call, the first of each page type.
    by_model: u32,
    /// Books extracted by replaying what the model learned.
    by_learned: u32,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    Foxhound::scope(ConnectOptions::default(), crawl_books).await
}

/// Declares the crawl, runs it to its end and reports what it spent and what it left unextracted.
async fn crawl_books(fox: &Foxhound) -> Result<(), Error> {
    let tally = RefCell::new(Tally::default());

    let mut crawl = Crawl::new();
    let home = crawl.label::<()>("home")?;
    let category = crawl.label::<()>("category")?;
    let book = crawl.label::<()>("book")?;
    let books = crawl.dataset::<BookRecord>("books")?;

    crawl.handle(&home, async |ctx: CrawlContext<()>| -> Result<(), Error> {
        let categories = ctx.page().locator(CATEGORY_LINKS);
        ctx.enqueue_links(&categories, &category, ()).limit(CATEGORY_LIMIT).await?;
        Ok(())
    })?;

    crawl.handle(&category, async |ctx: CrawlContext<()>| -> Result<(), Error> {
        let book_links = ctx.page().locator(BOOK_LINKS);
        ctx.enqueue_links(&book_links, &book, ()).await?;
        let next_page = ctx.page().locator(NEXT_PAGE_LINK);
        ctx.enqueue_links(&next_page, &category, ()).await?;
        Ok(())
    })?;

    crawl.handle(&book, async |ctx: CrawlContext<()>| -> Result<(), Error> {
        let url = ctx.request().url().to_owned();
        // A failed extraction, the allowance's included, returns its error: the request then ends
        // failed, or skipped with the model allowance as its reason.
        let step = ctx
            .page()
            .extract(BOOK_INSTRUCTION, Schema::<BookFacts>::of())
            .mode(ExtractMode::Compiled)
            .await?;
        let record = BookRecord {
            url,
            title: step.output.title,
            price: step.output.price,
            in_stock: step.output.in_stock,
            star_rating: step.output.star_rating,
        };
        ctx.push(&books, &record).await?;

        let mut counts = tally.borrow_mut();
        match step.step.source {
            StepSource::Model | StepSource::Repair => counts.by_model += 1,
            _ => counts.by_learned += 1,
        }
        Ok(())
    })?;

    // The model is set for the Run, and its spend is capped by the allowance: once a call would
    // exceed it, every later call is refused while the crawl goes on. One handler at a time means the
    // first book page learns the layout before any other compiles it again.
    let settings = Settings::new()
        .set(ai::MODEL, Some(ModelReference::from(MODEL_REFERENCE)))
        .set(ai::allowance::PRICE, Some(Money::new(MODEL_ALLOWANCE_USD, "USD")?))
        .set(settings::crawl::handler::CONCURRENCY, 1_u32);

    let seeds = [Seed::new(HOME_URL, &home, ())];
    let run = fox.start_crawl(&crawl, seeds).settings(settings).await?;
    let report = crawl.serve(&run).await?;

    println!(
        "requests: {} succeeded, {} failed, {} skipped, {} blocked",
        report.requests.succeeded, report.requests.failed, report.requests.skipped, report.requests.blocked,
    );
    {
        let counts = tally.borrow();
        println!(
            "books extracted by a model call: {}; from what the model learned: {}",
            counts.by_model, counts.by_learned,
        );
    }

    let usage = run.as_run().usage().await?;
    let spent = match &usage.model.price {
        Some(price) => format!("{} {}", price.amount(), price.currency()),
        None => "no price".to_owned(),
    };
    println!(
        "model calls: {}; tokens: {} (input {}, output {}); cost: {}",
        usage.model.calls,
        usage.model.tokens,
        usage.model.input_tokens,
        usage.model.output_tokens,
        spent,
    );
    for allowance in &usage.model_allowance {
        println!(
            "allowance {:?}: value {:?}, remaining {:?}, reached at {:?}",
            allowance.kind, allowance.value, allowance.remaining, allowance.reached_at,
        );
    }

    println!("books left unextracted:");
    let mut outcomes = run.outcomes(OutcomesFilter::default());
    while let Some(item) = outcomes.next().await {
        let outcome = item?;
        if outcome.request.label != "book" {
            continue;
        }
        match outcome.kind {
            CrawlRequestOutcomeKind::Succeeded { .. } => {}
            CrawlRequestOutcomeKind::Failed(error) => println!(
                "  {}: failed with {}: {}",
                outcome.request.url,
                error.code().as_str(),
                error.message(),
            ),
            CrawlRequestOutcomeKind::Skipped { reason, message } => println!(
                "  {}: skipped ({:?}){}",
                outcome.request.url,
                reason,
                message.map(|text| format!(": {text}")).unwrap_or_default(),
            ),
            CrawlRequestOutcomeKind::Blocked(cause) => {
                println!("  {}: blocked ({:?})", outcome.request.url, cause);
            }
            _ => {}
        }
    }

    Ok(())
}
