// A Dataset's handle carries its Record's type (ADR 0103): a push of a
// Record of another type does not compile.
use goodfetch_foxhound::{CrawlContext, Dataset};
use serde::Serialize;

#[derive(Serialize)]
struct Product {
    id: String,
}

#[derive(Serialize)]
struct Note {
    text: String,
}

async fn push(ctx: CrawlContext<()>, products: Dataset<Product>) {
    let note = Note { text: "out of stock".to_owned() };
    let _ = ctx.push(&products, &note).await;
}

fn main() {}
