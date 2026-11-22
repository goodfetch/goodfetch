// A Label without a Payload is declared with `()` (ADR 0082): a seed and an
// enqueue that give it a Payload do not compile.
use goodfetch_foxhound::{Crawl, CrawlContext, Error, Label, Seed};

fn seed(crawl: &mut Crawl<'_>) -> Result<Seed, Error> {
    let page = crawl.label::<()>("page")?;
    Ok(Seed::new("https://shop.test/", &page, 3))
}

async fn enqueue(ctx: CrawlContext<()>, page: Label<()>) {
    let _ = ctx.enqueue("https://shop.test/3", &page, "a listing").await;
}

fn main() {}
