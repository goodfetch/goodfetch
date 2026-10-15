//! Snippet for Capping a crawl's model use without stopping its Run (#513, ADR 0107), on golden
//! scenario 08.

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::ai;
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, CrawlRequestOutcomeKind, DatasetDeclaration, Ended, Error, ErrorCode, Foxhound, Money, OutcomeKind, OutcomesFilter, Page, Schema, Seed,
    Settings, SkipReason, StopReason,
};
use serde::{Deserialize, Serialize};

/// What the model extracts from a book's page.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookFields {
    title: String,
    /// Pounds, two decimals, kept as text.
    price: String,
    in_stock: u32,
    rating: u8,
}

/// A book's Record.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Book {
    #[serde(flatten)]
    fields: BookFields,
    url: String,
}

/// Scenario 08: the model is asked once per page type (the Plan), and stops being asked once the
/// crawl has spent 0.50 USD. The crawl goes on: books whose layout the Plan learned are still
/// extracted with no model call, and a book that needs the model is skipped with the reason
/// `model_allowance`.
async fn scenario_08(categories: &[&str]) -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;
    let category = crawl.label::<()>("category")?;
    let book = crawl.label::<()>("book")?;

    crawl.handle(&category, async |ctx: CrawlContext<()>| {
        ctx.enqueue_links(&ctx.page().locator("article.product_pod h3 a"), &book, ()).await?;
        ctx.enqueue_links(&ctx.page().locator("li.next a"), &category, ()).await?;
        Ok(())
    })?;

    crawl.handle(&book, async |ctx: CrawlContext<()>| {
        // Past the allowance, a step with no Plan entry or a Repair fails with
        // `model_allowance_reached`: returned, it ends this request `skipped` with the reason
        // `model_allowance`, not `failed`.
        let fields = ctx.page().extract("the book on this page", Schema::<BookFields>::of()).await?.output;
        ctx.push(&books, &Book { fields, url: ctx.page().url() }).await
    })?;

    let foxhound = Foxhound::connect(ConnectOptions::default().settings(Settings::new().set(ai::MODEL, Some("vercel/openai/gpt-5-mini".into())))).await?;
    let run = foxhound
        .start_crawl(&crawl, categories.iter().map(|url| Seed::new(url, &category, ())))
        .plan("books.plan.json") // learned once per page type, replayed with no model call (ADR 0091)
        // The Model allowance: not a Limit, so the crawl goes on once it is reached.
        .settings(Settings::new().set(ai::allowance::PRICE, Some(Money::new("0.50", "USD")?)))
        .await?;
    let report = crawl.serve(&run).await?; // `finished`, and `incomplete` when any book was left

    // The books left unextracted, from the Run's committed state.
    let mut skipped = run.outcomes(OutcomesFilter::default().kinds(&[OutcomeKind::Skipped]));
    while let Some(outcome) = skipped.next().await {
        let outcome = outcome?;
        if let CrawlRequestOutcomeKind::Skipped { reason: SkipReason::ModelAllowance, message } = &outcome.kind {
            let _ = (&outcome.request.url, message);
        }
    }
    let usage = &report.run.usage;
    let reached = usage.model_allowance.iter().map(|a| a.reached_at).collect::<Vec<_>>();
    let _ = (usage.model.tokens, &usage.model.price, reached, &report.run.completeness, &report.run.completeness_reasons);
    run.as_run().close().await?;
    foxhound.close().await
}

/// A handler that keeps a Record for every book catches the refusal: a skipped attempt keeps
/// nothing.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct KeptBook {
    url: String,
    book: Option<BookFields>,
    why: Option<String>,
}

fn keep_all() -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let kept = crawl.dataset(DatasetDeclaration::<KeptBook>::new("books").key(&["url"]))?;
    let book = crawl.label::<()>("book")?;
    crawl.handle(&book, async |ctx: CrawlContext<()>| {
        let url = ctx.page().url();
        match ctx.page().extract("the book on this page", Schema::<BookFields>::of()).await {
            Ok(step) => ctx.push(&kept, &KeptBook { url, book: Some(step.output), why: None }).await,
            Err(error) if error.code() == ErrorCode::MODEL_ALLOWANCE_REACHED => {
                ctx.push(&kept, &KeptBook { url, book: None, why: Some("the Model allowance was reached".into()) }).await
            }
            Err(error) => Err(error),
        }
    })?;
    drop(crawl); // served as in `scenario_08`
    Ok(())
}

/// An Agent's task ends as a value when the allowance is reached, and the Run goes on.
async fn agent_stops(page: &Page) -> Result<Option<String>, Error> {
    let result = page.agent("add the cheapest poetry book to the basket").await?;
    if let Ended::Stopped(StopReason::ModelAllowance(source)) = &result.ended {
        return Ok(Some(source.setting.clone()));
    }
    Ok(None)
}

fn main() {
    let _ = (scenario_08, keep_all, agent_stops);
    let _ = schemars::schema_for!(Book);
}
