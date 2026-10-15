//! Snippet for Crawl output in code (#479, ADR 0103), on golden scenarios 05, 07 and 12.

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRun, Dataset, DatasetDeclaration, Error, ExportFormat, Foxhound, Seed, Settings,
    SinkConfig, SinkWriteMode,
};
use serde::{Deserialize, Serialize};

/// A book, its price kept as exact text.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Book {
    title: String,
    price: String,
    url: String,
    category: String,
}

/// The program's own state, kept in the Run's Key-value store.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Cursor {
    last_page: u32,
}

/// The Payload of a category request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct CategoryPayload {
    name: String,
}

/// Scenario 05 and 12: Datasets are declared on the crawl, as its Labels are, with no default
/// Dataset; the key identifies a book collected twice, and Sinks upsert on it. A handler stages its
/// Records, and its Key-value store writes, with its attempt.
async fn scenario_05() -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;
    let skipped = crawl.dataset::<serde_json::Map<String, serde_json::Value>>("skipped")?; // any JSON object
    let category = crawl.label::<CategoryPayload>("category")?;

    crawl.handle(&category, async |ctx: CrawlContext<CategoryPayload>| {
        let name = ctx.request().payload().name.clone();
        for pod in ctx.page().locator("article.product_pod").all().await? {
            let url = pod.locator("h3 a").get_attribute("href").await?.unwrap_or_default();
            let price = pod.locator(".price_color").text_content().await?;
            let Some(pounds) = price.strip_prefix('£') else {
                let mut why = serde_json::Map::new();
                why.insert("url".into(), url.into());
                ctx.push(&skipped, &why).await?;
                continue;
            };
            // Committed with the attempt, numbered, or discarded if it fails, retries or escalates (P18).
            let title = pod.locator("h3 a").get_attribute("title").await?.unwrap_or_default();
            ctx.push(&books, &Book { title, price: pounds.to_owned(), url, category: name.clone() }).await?;
        }
        // Staged too, through the attempt's `run`: the cursor advances only if this page's Records commit.
        ctx.run().key_value_store().set(&format!("cursor/{name}"), &Cursor { last_page: ctx.request().depth() }).await?;
        ctx.enqueue_links(&ctx.page().locator("li.next a"), ctx.request().label(), CategoryPayload { name }).await?;
        Ok(())
    })?;

    let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    let seed = Seed::new(
        "https://books.toscrape.com/catalogue/category/books/travel_2/index.html",
        &category,
        CategoryPayload { name: "Travel".into() },
    );
    // A Sink is a setting: from code, the project file, the CLI or a Recipe alike.
    let sinks = Settings::new().sink("warehouse", SinkConfig::new("postgres", ["books"]).mode(SinkWriteMode::Upsert));
    let run = foxhound.start_crawl(&crawl, [seed]).settings(sinks).await?;
    let _report = crawl.serve(&run).await?; // the Run report once it ends (ADR 0102)

    let cursor: Option<Cursor> = run.key_value_store().get("cursor/Travel").await?;
    let csv = run.export(&books, ExportFormat::Csv).await?;
    csv.save_as("books.csv").await?;
    let _ = (cursor, csv.records(), csv.last_number(), csv.sha256());
    run.as_run().close().await?;
    foxhound.close().await
}

/// Scenario 12: a service reads Records as they commit and resumes from the last number it handled.
/// The reader is a cursor over run state: slow or paused, it never slows the crawl.
async fn follow(run: &CrawlRun, books: &Dataset<Book>, last_handled: u64) -> Result<u64, Error> {
    let mut records = run.records_after(books, last_handled);
    let mut last = last_handled;
    while let Some(record) = records.next().await {
        match record {
            Ok(record) => last = record.number,
            Err(error) => {
                // A Record `Book`'s `Deserialize` now rejects arrives with its number.
                if let Some(details) = error.details::<goodfetch_foxhound::RecordInvalid>() {
                    let _ = details.number;
                }
                return Err(error);
            }
        }
    }
    Ok(last)
}

fn main() {
    let _ = (scenario_05, follow);
    let _ = schemars::schema_for!(Book);
}
