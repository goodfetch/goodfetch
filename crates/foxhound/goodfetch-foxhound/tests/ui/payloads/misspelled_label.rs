// A Label is named by its handle, a value (ADR 0093): a misspelled handle
// names nothing, so the seed does not compile.
use goodfetch_foxhound::{Crawl, Error, Seed};

fn seed(crawl: &mut Crawl<'_>) -> Result<Seed, Error> {
    let shelf = crawl.label::<u32>("shelf")?;
    let _ = shelf;
    Ok(Seed::new("https://shop.test/", &shelff, 3))
}

fn main() {}
