//! Snippet for Decoding a response the same way on every tier (#550, ADR 0119).

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::{crawl, export, fetch};
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRequestOutcomeKind, DatasetDeclaration, Error, ExportFormat, Foxhound, LabelDeclaration, OutcomeKind, OutcomesFilter, Seed, Settings,
    SkipReason,
};
use serde::{Deserialize, Serialize};

/// A notice of the listing pages.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Notice {
    title: String,
    url: String,
}

/// A PDF the crawl kept.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct File {
    url: String,
    content_type: String,
    size: u64,
}

async fn decoding() -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let notices = crawl.dataset(DatasetDeclaration::<Notice>::new("notices"))?;
    let files = crawl.dataset(DatasetDeclaration::<File>::new("files"))?;
    // HTML pages, the built-in `crawl.content_types`. The site's legacy section declares no
    // charset, so its Label forces one rather than escalating each page to the browser.
    let listing = crawl.label(LabelDeclaration::<()>::new("listing").settings(Settings::new().set(fetch::ENCODING, Some("shift_jis".to_owned()))))?;
    // The Label takes PDFs too: their bytes are in `ctx.response()`, and their page holds no document.
    let attachment = crawl.label(
        LabelDeclaration::<()>::new("attachment")
            .settings(Settings::new().set(crawl::CONTENT_TYPES, vec!["text/html".to_owned(), "application/pdf".to_owned()])),
    )?;

    crawl.handle(&listing, async |ctx: CrawlContext<()>| {
        let title = ctx.page().locator("h1").text_content().await?;
        ctx.push(&notices, &Notice { title, url: ctx.page().url() }).await?;
        ctx.enqueue_links(&ctx.page().locator("a.attachment"), &attachment, ()).await?;
        Ok(())
    })?;

    crawl.handle(&attachment, async |ctx: CrawlContext<()>| {
        let content_type = ctx.response().headers().get("content-type").cloned().unwrap_or_default();
        if content_type.starts_with("application/pdf") {
            // `ctx.page().locator(…)` here would fail with `response_not_document` at once, on every tier.
            let body = ctx.response().body().await?;
            let size = u64::try_from(body.len())?;
            ctx.run().key_value_store().set_bytes(&format!("pdf/{}", ctx.page().url()), body, "application/pdf").await?;
            ctx.push(&files, &File { url: ctx.page().url(), content_type, size }).await?;
            return Ok(());
        }
        // `text()` decodes with the document's own encoding, so it agrees with `page.content()`.
        let _ = ctx.response().text().await?.len();
        Ok(())
    })?;

    let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    let run = foxhound
        .start_crawl(&crawl, [Seed::new("https://example.org/notices/", &listing, ())])
        .settings(Settings::new().set(export::csv::BOM, true)) // the CSV Export opens in a spreadsheet
        .await?;
    let _report = crawl.serve(&run).await?;
    let mut skipped = run.outcomes(OutcomesFilter::default().kinds(&[OutcomeKind::Skipped]));
    while let Some(outcome) = skipped.next().await {
        let outcome = outcome?;
        // A ZIP or an image under `attachment`: skipped before its body is read.
        if let CrawlRequestOutcomeKind::Skipped { reason: SkipReason::ContentType, message } = &outcome.kind {
            let _ = (&outcome.request.url, message);
        }
    }
    let csv = run.export(&notices, ExportFormat::Csv).await?; // UTF-8, with a BOM under `export.csv.bom`
    csv.save_as("notices.csv").await?;
    run.as_run().close().await?;
    foxhound.close().await
}

fn main() {
    let _ = decoding;
    let _ = schemars::schema_for!(Notice);
}
