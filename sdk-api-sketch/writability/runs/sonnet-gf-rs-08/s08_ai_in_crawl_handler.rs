//! Crawls the first three categories of books.toscrape.com and every book they list, following
//! the categories' "next" pages. A language model reads each book's title, price, stock and star
//! rating through `page.extract`, in its default compiled mode: the model is asked once for the
//! book page layout, the Run's Plan keeps what it wrote, and every later book page is read from the
//! Plan with no model call (a page that no longer matches is repaired once by the model). The Run's
//! Model allowance stops all model use once the crawl has spent 0.50 USD: the crawl then finishes
//! without the model, and the books left unextracted are reported with the reason.

use std::cell::RefCell;
use std::collections::BTreeMap;

use goodfetch_foxhound::settings::ai;
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, DatasetDeclaration, Error, ErrorCode, Foxhound,
    ModelReference, Money, Schema, Seed, Settings, StepSource,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const START_URL: &str = "https://books.toscrape.com/";
const CATEGORY_COUNT: u32 = 3;
const ALLOWANCE_USD: &str = "0.50";
const DEFAULT_MODEL: &str = "vercel/anthropic/claude-sonnet-4.5";
const PLAN_FILE: &str = "books-plan.json";

/// One instruction for every book page, so the Plan holds a single entry for the layout.
const EXTRACT_INSTRUCTION: &str = "Read this book's detail page: its title, its price as a number \
without the currency symbol, the number of copies in stock (from the availability line), and its \
star rating as a whole number from 1 to 5.";

/// What the model reads off a book page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct BookFields {
    /// The book's title.
    title: String,
    /// The price as a number, in pounds, with no currency symbol.
    price: f64,
    /// The number of copies in stock, from the availability line.
    in_stock: u32,
    /// The star rating, a whole number from 1 to 5.
    rating: u8,
}

/// One Record of the `books` Dataset.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Book {
    /// The book page's URL.
    url: String,
    /// The book's title.
    title: String,
    /// The price as a number, in pounds.
    price: f64,
    /// The number of copies in stock.
    in_stock: u32,
    /// The star rating, 1 to 5.
    rating: u8,
}

/// What the handlers tally while the crawl runs.
#[derive(Default)]
struct Tally {
    /// Books extracted, by how the step was resolved (plan, model, repair, journal, other).
    extracted: BTreeMap<&'static str, u64>,
    /// Books whose extraction failed or was refused, by URL, with the reason.
    unextracted: BTreeMap<String, String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = std::env::var("GOODFETCH_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());

    Foxhound::scope(ConnectOptions::default(), async |fox: &Foxhound| -> Result<(), Error> {
        let tally = RefCell::new(Tally::default());
        let tally = &tally;

        let mut crawl = Crawl::new();
        let home = crawl.label::<()>("home")?;
        let category = crawl.label::<()>("category")?;
        let book = crawl.label::<()>("book")?;
        let books = crawl.dataset(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;

        // The side list: the first three categories, not the "Books" root above them.
        let category_for_home = category.clone();
        crawl.handle(&home, async move |ctx: CrawlContext<()>| -> Result<(), Error> {
            let links = ctx.page().locator("div.side_categories > ul > li > ul > li > a");
            ctx.enqueue_links(&links, &category_for_home, ()).limit(CATEGORY_COUNT).await?;
            Ok(())
        })?;

        // Every book of a category page, then its "next" page, of the same Label.
        let book_for_category = book.clone();
        crawl.handle(&category, async move |ctx: CrawlContext<()>| -> Result<(), Error> {
            let page = ctx.page();
            let listed = page.locator("article.product_pod h3 a");
            ctx.enqueue_links(&listed, &book_for_category, ()).await?;
            let next = page.locator("ul.pager li.next a");
            ctx.enqueue_links(&next, ctx.request().label(), ()).await?;
            Ok(())
        })?;

        // A book page: extracted by the model, from the Plan once it holds the layout.
        let books_dataset = books.clone();
        crawl.handle(&book, async move |ctx: CrawlContext<()>| -> Result<(), Error> {
            let url = ctx.request().url().to_owned();
            let extracted = ctx
                .page()
                .extract::<BookFields>(EXTRACT_INSTRUCTION, Schema::of())
                .await;
            match extracted {
                Ok(result) => {
                    let source = match result.step.source {
                        StepSource::Plan => "plan",
                        StepSource::Model => "model",
                        StepSource::Repair => "repair",
                        StepSource::Journal => "journal",
                        _ => "other",
                    };
                    let fields = result.output;
                    let record = Book {
                        url: url.clone(),
                        title: fields.title,
                        price: fields.price,
                        in_stock: fields.in_stock,
                        rating: fields.rating,
                    };
                    ctx.push(&books_dataset, &record).await?;
                    let mut tally = tally.borrow_mut();
                    *tally.extracted.entry(source).or_insert(0) += 1;
                    tally.unextracted.remove(&url);
                    Ok(())
                }
                Err(error) => {
                    let reason = if error.code() == ErrorCode::MODEL_ALLOWANCE_REACHED {
                        "the model allowance was reached and the Plan held no entry for this page"
                            .to_owned()
                    } else {
                        format!("{}: {}", error.code().as_str(), error.message())
                    };
                    tally.borrow_mut().unextracted.insert(url, reason);
                    // Returned as is: a reached allowance ends the request `skipped` and the
                    // crawl goes on without the model; any other error follows the retry policy.
                    Err(error)
                }
            }
        })?;

        let settings = Settings::new()
            .set(ai::MODEL, ModelReference::from(model.as_str()))
            .set(ai::allowance::PRICE, Money::new(ALLOWANCE_USD, "USD")?);

        let run = fox
            .start_crawl(&crawl, [Seed::new(START_URL, &home, ())])
            .id("books-toscrape-ai")
            .plan(PLAN_FILE)
            .settings(settings)
            .await?;
        let report = crawl.serve(&run).await?;

        let tally = tally.borrow();
        let extracted_total: u64 = tally.extracted.values().sum();
        println!("Run {}: {:?}, {:?}", report.run.id, report.run.lifecycle, report.run.completeness);
        println!(
            "Requests: {} succeeded, {} failed, {} skipped, {} blocked, {} pending",
            report.requests.succeeded,
            report.requests.failed,
            report.requests.skipped,
            report.requests.blocked,
            report.requests.pending
        );
        for (reason, count) in &report.requests.by_skip_reason {
            println!("  skipped {count} for {reason:?}");
        }
        for (dataset, totals) in &report.records {
            println!("Dataset {dataset}: {} committed, {} rejected", totals.committed, totals.rejected);
        }
        println!("Books extracted: {extracted_total}");
        for (source, count) in &tally.extracted {
            println!("  {source}: {count}");
        }

        println!("Books left unextracted: {}", tally.unextracted.len());
        for (url, reason) in &tally.unextracted {
            println!("  {url}: {reason}");
        }

        let usage = &report.run.usage.model;
        println!(
            "Model usage: {} calls, {} tokens ({} in, {} out)",
            usage.calls, usage.tokens, usage.input_tokens, usage.output_tokens
        );
        match &usage.price {
            Some(price) => println!("Cost: {} {}", price.amount(), price.currency()),
            None => println!("Cost: unknown (the price table lacks a price for the model)"),
        }
        for allowance in &report.run.usage.model_allowance {
            println!(
                "Allowance {:?}: remaining {:?}, reached: {}",
                allowance.kind,
                allowance.remaining,
                allowance.reached_at.is_some()
            );
        }
        Ok(())
    })
    .await?;
    Ok(())
}
