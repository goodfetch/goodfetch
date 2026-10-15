//! Snippet for Typed data: schemas, Records and Payloads (#469), on golden scenarios 02 and 05.

use goodfetch_sdk_sketch::schemars::{self, JsonSchema};
use goodfetch_sdk_sketch::{CrawlContext, Dataset, Error, Label, Page, Schema};
use serde::{Deserialize, Serialize};

// Provisional, its owner's to decide: `page.extract` is ADR 0091's. A crawl's Labels, handlers and
// `enqueue` are ADR 0093's, its Datasets and `push` ADR 0103's.
async fn enqueue<P: Serialize>(url: &str, label: &Label<P>, payload: &P) -> Result<(), Error> {
    let _ = (url, label, payload);
    unimplemented!()
}

/// Scenario 02: a typed struct; the price is an exact decimal kept as text, never a float.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Book {
    title: String,
    /// Pounds, two decimals.
    price: String,
    in_stock: u32,
    /// One to five.
    #[schemars(range(min = 1, max = 5))]
    rating: u8,
    description: String,
}

async fn typed_extract(page: &Page) -> Result<String, Error> {
    // Checked by JSON Schema, then by serde itself, inside the retry loop.
    let book = page.extract("the book on this page", Schema::<Book>::of()).await?.output;
    Ok(format!("{}: £{}", book.title, book.price))
}

async fn raw_schema(page: &Page) -> Result<serde_json::Value, Error> {
    // The library-free way down: a hand-written JSON Schema gives a `serde_json::Value`.
    let doc = serde_json::json!({ "type": "object", "properties": { "price": { "type": "string" } } });
    Ok(page.extract("the price", Schema::from_json(doc)).await?.output)
}

/// Scenario 05: the category name travels as a typed Payload, apart from the engine's own fields.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookPayload {
    category: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookRecord {
    #[serde(flatten)]
    book: Book,
    url: String,
    category: String,
}

async fn on_category_page(book_page: &Label<BookPayload>, category: &str, book_urls: &[String]) -> Result<(), Error> {
    for url in book_urls {
        enqueue(url, book_page, &BookPayload { category: category.to_owned() }).await?;
    }
    // Another type as the Payload does not compile: `enqueue(url, book_page, &Book { .. })` expects
    // `&BookPayload`, so a wrong Payload never reaches the handler.
    Ok(())
}

async fn on_book_page(ctx: &CrawlContext<BookPayload>, books: &Dataset<BookRecord>) -> Result<(), Error> {
    let book = ctx.page().extract("the book on this page", Schema::<Book>::of()).await?.output;
    let record = BookRecord { book, url: ctx.page().url(), category: ctx.request().payload().category.clone() };
    ctx.push(books, &record).await
}

fn main() {
    let _ = (typed_extract, raw_schema, on_category_page, on_book_page);
    let _ = schemars::schema_for!(Book);
}
