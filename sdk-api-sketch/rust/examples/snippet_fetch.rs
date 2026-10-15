//! Snippet for Redirect limits, truncated bodies and transport failures on the fetch tiers (#549,
//! ADR 0118).

use std::time::Duration;

use goodfetch_foxhound::settings::{crawl, fetch, politeness};
use goodfetch_foxhound::{
    BlockCause, Blocked, ConnectOptions, Crawl, CrawlContext, CrawlRequestOutcomeKind, Error, ErrorCode, Foxhound,
    LabelDeclaration, OutcomeKind, OutcomesFilter, ResponseTruncated, ResponseUnavailable, Seed, Settings,
    TooManyRedirects, error_code,
};

/// A link checker reads each redirect itself: a redirect status listed in `crawl.handle_statuses`
/// is not followed, the one way not to follow, so the handler gets the 301 with its `Location`.
/// Every other crawl follows Fetch's redirects on every tier, at most `fetch.redirect.max`, and a
/// refresh stub's target as one more hop; a loop, a cut body or a long `Retry-After` is a typed
/// outcome. A cut body never reaches a handler, so `Response` has no flag to forget to check (G1).
async fn fetch_edges() -> Result<(), Error> {
    let mut links = Crawl::new();
    let link = links.label(
        LabelDeclaration::<()>::new("link")
            .settings(Settings::new().set(crawl::HANDLE_STATUSES, vec![301, 302, 303, 307, 308, 404])),
    )?;
    links.handle(&link, async |ctx: CrawlContext<()>| {
        let response = ctx.response();
        if (300..400).contains(&response.status()) {
            let _ = (response.url(), response.headers().get("location"));
        }
        Ok(())
    })?;

    let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    let run = foxhound
        .start_crawl(&links, [Seed::new("https://example.com/", &link, ())])
        .settings(
            Settings::new()
                .set(fetch::redirect::MAX, 10_u32)
                .set(fetch::refresh::MAX_DELAY, Some(Duration::ZERO)) // instant refreshes only
                .set(politeness::retry_after::MAX, Duration::from_secs(3600)), // a site's maintenance hour
        )
        .await?;
    let _report = links.serve(&run).await?;

    let kinds = [OutcomeKind::Failed];
    let mut outcomes = run.outcomes(OutcomesFilter::default().kinds(&kinds));
    while let Some(outcome) = outcomes.next().await {
        let CrawlRequestOutcomeKind::Failed(error) = &outcome?.kind else { continue };
        if error.code() == ErrorCode::TOO_MANY_REDIRECTS {
            let _chain = error.details::<TooManyRedirects>().map(|d| d.chain);
        } else if error.code() == ErrorCode::RESPONSE_TRUNCATED {
            let _received = error.details::<ResponseTruncated>().map(|d| d.received_bytes);
        } else if error.code() == error_code::RESPONSE_UNAVAILABLE {
            // A failure, so `retry_failed()` re-admits it once the site is back.
            let _until = error.details::<ResponseUnavailable>().and_then(|d| d.retry_after);
            let _ = error.fix();
        }
    }
    run.as_run().close().await?;
    foxhound.close().await
}

/// A Collie page follows the browser's 20 redirects and never waits on a `Retry-After`: the
/// program directs its own navigation, and reads the site's time from the error.
async fn collie_page(foxhound: &Foxhound) -> Result<(), Error> {
    let run = foxhound.start().await?;
    let page = run.new_page().await?;
    let navigated = page.goto("https://example.com/").await;
    match navigated {
        Ok(_) => {
            let api = page.request().get("https://example.com/api/items").max_redirects(0).await?; // the 3xx itself
            let _ = api.status();
        }
        Err(error) if error.code() == ErrorCode::RESPONSE_TRUNCATED && error.retryable() => {
            page.reload().await?; // the program's own retry
        }
        Err(error) if error.code() == ErrorCode::BLOCKED => {
            let blocked = error.details::<Blocked>();
            if let Some(Blocked { block_cause: BlockCause::RateLimited, retry_after, .. }) = blocked {
                let _until = retry_after;
            }
        }
        Err(error) => return Err(error),
    }
    run.close().await
}

fn main() {
    let _ = (fetch_edges, collie_page);
}
