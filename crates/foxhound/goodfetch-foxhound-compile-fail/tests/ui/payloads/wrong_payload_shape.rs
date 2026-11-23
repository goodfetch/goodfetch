// A Label's handle carries its Payload's type (ADR 0082): a seed and an
// enqueue whose Payload has another type than their Label's do not compile.
use goodfetch_foxhound::{Crawl, CrawlContext, Error, Label, Seed};

fn seed(crawl: &mut Crawl<'_>) -> Result<Seed, Error> {
    let shelf = crawl.label::<u32>("shelf")?;
    Ok(Seed::new("https://shop.test/", &shelf, "three"))
}

async fn enqueue(ctx: CrawlContext<()>, shelf: Label<u32>) {
    let _ = ctx.enqueue("https://shop.test/3", &shelf, "three").await;
}

fn main() {}
