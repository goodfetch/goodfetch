//! Golden scenario 02, "Extract a typed object with a chosen model" (ADRs 0082, 0091, 0101).

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::{
    Collie, ConnectOptions, Error, Expected, ExtractionIncomplete, ExtractionInvalid, Schema,
    expect,
};
use serde::Deserialize;

/// The book, typed as the task asks.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Book {
    /// Its title.
    pub title: String,
    /// Pounds, an exact decimal kept as text.
    #[schemars(regex(pattern = r"^\d+\.\d{2}$"))]
    pub price: String,
    /// Copies in stock.
    pub in_stock: u32,
    /// One to five stars.
    #[schemars(range(min = 1, max = 5))]
    pub rating: u8,
    /// Its description.
    pub description: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // A Model reference names its provider, then that provider's model ID: a model Ollama serves
    // locally is one more provider, priced zero at its default address (ADR 0101).
    let model = if std::env::args().any(|arg| arg == "--local") {
        "ollama/qwen3:8b"
    } else {
        "anthropic/claude-sonnet-4-5"
    };
    // Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic. The scope closes the
    // connection, and the Run with it, on every return.
    Collie::scope(ConnectOptions::default(), async |collie| {
        let run = collie.start().await?;
        let page = run.new_page().await?;
        page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html")
            .await?;
        // Not another page, nor an error page.
        expect(&page)
            .to_have_title(Expected::regex("A Light in the Attic"))
            .await?;
        // Every answer is checked against Book; a provider that rejects native structured output
        // gets a forced tool call, then the schema in the prompt (`ai.structured_output: auto`).
        let result = page
            .extract("the book on this page", Schema::<Book>::of())
            .model(model)
            .await
            .inspect_err(say_which_field)?;
        let usage = &result.usage;
        let price = usage.price.as_ref().map_or("unpriced".to_owned(), |price| {
            format!("{} {}", price.amount(), price.currency())
        });
        println!(
            "{:?} {} tokens ({} in, {} out), {price}",
            result.output, usage.tokens, usage.input_tokens, usage.output_tokens
        );
        let _ = run.finish().await?;
        Ok(())
    })
    .await
}

/// Names the fields a failed extraction missed or mistyped; the error's `usage()` says what the
/// failed attempts spent.
fn say_which_field(error: &Error) {
    if let Some(incomplete) = error.details::<ExtractionIncomplete>() {
        let fields: Vec<_> = incomplete.missing.iter().map(|field| &field.path).collect();
        eprintln!("Not on the page: {fields:?}");
    }
    if let Some(invalid) = error.details::<ExtractionInvalid>() {
        for issue in &invalid.issues {
            eprintln!(
                "Not of the expected type: {:?}: {}",
                issue.path, issue.message
            );
        }
    }
}
