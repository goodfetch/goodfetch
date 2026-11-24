// A Dataset is named by its handle, a value (ADR 0103): a misspelled
// handle names nothing, so the push does not compile.
use goodfetch_foxhound::{CrawlContext, Dataset};
use serde::Serialize;

#[derive(Serialize)]
struct Product {
    id: String,
}

async fn push(ctx: CrawlContext<()>, products: Dataset<Product>) {
    let product = Product { id: "P-1".to_owned() };
    let _ = products;
    let _ = ctx.push(&prodcts, &product).await;
}

fn main() {}
