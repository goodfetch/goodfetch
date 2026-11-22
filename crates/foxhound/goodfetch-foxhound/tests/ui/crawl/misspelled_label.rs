// A Label is a handle the program declares once and names at each use, not
// a string it spells again (ADR 0093): a misspelled handle is an unknown
// name, so `cargo check` fails where a misspelled string would fail the
// start of the Run.
use goodfetch_foxhound::{Crawl, CrawlContext, Seed};

#[derive(serde::Serialize, serde::Deserialize)]
struct Visit {
    depth: u32,
}

fn main() {
    let mut crawl = Crawl::new();
    let page = crawl.label::<Visit>("page").unwrap();
    let _ = crawl.handle(&pgae, |_ctx: CrawlContext<Visit>| async { Ok(()) });
    let _seed = Seed::new("https://shop.test/", &page, Visit { depth: 0 });
}
