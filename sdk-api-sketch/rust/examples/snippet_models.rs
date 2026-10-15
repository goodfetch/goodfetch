//! Snippet for Models, usage and cost in the SDK API (#475, ADR 0101), on golden scenarios 02, 08
//! and 11.

use std::time::Duration;

use goodfetch_sdk_sketch::schemars::{self, JsonSchema};
use goodfetch_sdk_sketch::settings::{ai, timeout};
use goodfetch_sdk_sketch::{
    ConnectOptions, CrawlRun, Error, ErrorCode, ErrorDetails, Goodfetch, LimitAmount, Limits,
    ModelDeclaration, ModelFormat, ModelProvider, ModelResponse, ModelUsageOf, Money, Page,
    ReportedTokens, Schema, Settings, StructuredOutput, StructuredOutputStrategy,
    UnavailableReason,
};
use serde::Deserialize;

/// A book, as golden scenario 02 reads it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Book {
    /// Its title.
    pub title: String,
    /// Its price in pounds, kept exact as text.
    pub price: String,
    /// Copies in stock.
    pub in_stock: u32,
    /// One to five stars.
    pub rating: u8,
    /// Its description.
    pub description: String,
}

/// The program's own logging, through `tracing` in a real program.
fn log(message: &str, calls: u64, tokens: u64, price: Option<&Money>) {
    let _ = (
        message,
        calls,
        tokens,
        price.map(|p| (p.amount(), p.currency())),
    );
}

/// Golden scenario 02, "Extract a typed object with a chosen model". The model is a Model
/// reference: a provider's name, a slash, and that provider's own model ID. The connection's setting
/// serves every call; one call may name its own, here a local model served by Ollama, priced zero at
/// its default address.
async fn typed_book(local: bool) -> Result<Option<Book>, Error> {
    // The key: env:ANTHROPIC_API_KEY, sent only to Anthropic.
    let settings = Settings::new().set(ai::MODEL, Some("anthropic/claude-sonnet-4-5".into()));
    let gf = Goodfetch::connect(ConnectOptions::default().settings(settings)).await?;
    let run = gf.collie().start().await?;
    let page = run.new_page().await?;
    page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html")
        .await?;
    let extract = page.extract("the book on this page", Schema::<Book>::of());
    let extract = if local {
        extract.model("ollama/qwen3:8b")
    } else {
        extract
    };
    let book = match extract.await {
        Ok(result) => {
            // Always there, priced from the price table.
            log(
                "extracted",
                result.usage.calls,
                result.usage.tokens,
                result.usage.price.as_ref(),
            );
            Some(result.output)
        }
        Err(error) if *error.code() == ErrorCode::ExtractionIncomplete => None,
        Err(error) => {
            // An invalid answer names the model and the strategy; what the attempts cost is on the error too.
            if let (Some(ErrorDetails::ExtractionInvalid(invalid)), Some(spent)) =
                (error.details(), error.usage())
            {
                let _ = (
                    &invalid.model,
                    invalid.strategy == StructuredOutputStrategy::Tool,
                );
                log(
                    "never fitted the schema",
                    spent.calls,
                    spent.tokens,
                    spent.price.as_ref(),
                );
            }
            return Err(error);
        }
    };
    run.close().await?;
    gf.close().await?;
    Ok(book)
}

/// The program's own Model provider (ADR 0033's SDK-handler form): calls arrive as a stream, answered
/// with Goodfetch's own types, no provider crate.
async fn in_house_provider(gf: &Goodfetch) -> Result<(), Error> {
    let mut calls = gf.model_requests("inhouse"); // "inhouse/<anything>" reaches it while this is open
    while let Some(call) = calls.next().await {
        let call = call?;
        let _ = &call.request().messages; // the program's own inference service answers them
        call.answer(ModelResponse::text(
            "{}".to_owned(),
            ReportedTokens::new(812, 64),
        ));
    }
    Ok(())
}

/// A gateway, a declared OpenAI-compatible server, and capabilities declared rather than guessed.
/// Prices are set only in configuration.
async fn providers() -> Result<(), Error> {
    let lab = ModelProvider::new(ModelFormat::OpenAi, "http://gpu-box.internal:8000/v1")
        .api_key("env:LAB_VLLM_KEY".parse()?) // a Secret reference, sent only to the base URL
        .model(
            "qwen3-vl-32b",
            ModelDeclaration::new()
                .image_input()
                .context_window(131_072)
                .strategies(&[
                    StructuredOutputStrategy::Native,
                    StructuredOutputStrategy::Prompt,
                ]),
        );
    let settings = Settings::new()
        .set(ai::MODEL, Some("vercel/openai/gpt-5-mini".into())) // the Vercel AI Gateway (ADR 0076)
        .set(ai::STRUCTURED_OUTPUT, StructuredOutput::Auto)
        .model_provider("lab", lab)
        .set(timeout::MODEL, Some(Duration::from_secs(120))); // a slow server
    let gf = Goodfetch::connect(ConnectOptions::default().settings(settings)).await?;
    let run = gf
        .collie()
        .start()
        .settings(Settings::new().set(ai::MODEL, Some("lab/qwen3-vl-32b".into())))
        .await?;
    let page = run.new_page().await?;
    page.goto("https://books.toscrape.com/").await?;
    page.act("open the Travel category").await?;
    Ok(())
}

/// Run totals, what remains of each Limit, and the uncapped mark (ADR 0041). The price Limit and a
/// usage's price are the same `Money`.
async fn capped_run(gf: &Goodfetch) -> Result<(), Error> {
    let limits = Limits::new()
        .price(Money::new("0.20", "USD")?)
        .model_calls(40);
    let run = gf.collie().start().limits(limits).await?;
    let page = run.new_page().await?;
    page.goto("https://books.toscrape.com/").await?;
    page.act("open the Travel category").await?;

    let usage = run.usage().await?;
    log(
        "so far",
        usage.model.calls,
        usage.model.tokens,
        usage.price.as_ref(),
    );
    let _ = (usage.browser_time, usage.uncapped);
    for limit in &usage.limits {
        if let LimitAmount::Price { remaining, .. } = &limit.amount {
            log("price remaining", 0, 0, Some(remaining));
        }
    }
    Ok(())
}

/// A provider outage is retryable and says when to come back; a rejected key or an exhausted
/// credit is not.
async fn with_outage(page: &Page) -> Result<(), Error> {
    match page.act("add the first book to the basket").await {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(ErrorDetails::ModelUnavailable(unavailable)) = error.details() {
                if unavailable.reason == UnavailableReason::RateLimited {
                    let _ = (unavailable.retry_after, &unavailable.failure.model);
                    return Ok(());
                }
            }
            let _fatal = matches!(
                error.code(),
                ErrorCode::ModelUnauthorized | ErrorCode::ModelQuotaExhausted
            );
            Err(error)
        }
    }
}

/// Golden scenario 08's Usage report. Its "stop model use, finish without the model" is "Capping a
/// crawl's model use without stopping its Run" (#513): here the price Limit stops the crawl.
async fn report_spend(run: &CrawlRun) -> Result<(), Error> {
    let usage = run.as_run().usage().await?;
    for ModelUsageOf {
        model,
        calls,
        tokens,
        price,
        ..
    } in &usage.model.by_model
    {
        log(model.as_str(), *calls, *tokens, price.as_ref());
    }
    let _ = (usage.price, usage.uncapped);
    Ok(())
}

fn main() {
    let _ = (
        typed_book,
        in_house_provider,
        providers,
        capped_run,
        with_outage,
        report_spend,
    );
    let _ = schemars::schema_for!(Book);
}
