//! The handler of Foxhound's sample operation, `sample.crawl`, which carries
//! the catalog's pipeline until the crawl areas replace it (spec #130); it
//! goes with the sample entries.

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::foxhound::{ENTRIES, SampleCrawlParams, SampleCrawlResult};

/// The sample operations only Foxhound carries.
pub(crate) fn contribution() -> Contribution {
    let crawl = ENTRIES
        .operations
        .iter()
        .find(|operation| operation.name == "sample.crawl")
        .expect("invariant: Foxhound's sample entries hold `sample.crawl`");
    Contribution::new("foxhound-sample").with_handler(Handler::new(crawl, visit))
}

async fn visit(params: SampleCrawlParams, _: CallContext) -> Result<SampleCrawlResult, CallError> {
    Ok(SampleCrawlResult {
        visited: params.pages,
    })
}
