//! Extracts one book from its catalogue page as a typed object. A language model reads the page
//! (no hand-written selectors), and the model is chosen by name on the command line:
//! `s02_typed_extract <provider/model>`, such as `ollama/qwen3:8b` for a local model or
//! `openai/<model id>` for a hosted one. Prints the book, then the tokens used and their cost.

use std::process::ExitCode;

use goodfetch_collie::schemars::JsonSchema;
use goodfetch_collie::settings::ai::MODEL;
use goodfetch_collie::{
    Collie, ConnectOptions, Error, ExtractMode, ExtractionIncomplete, ExtractionInvalid,
    ModelReference, ModelUsage, PathSegment, Run, Schema, Settings, StepResult,
};
use serde::Deserialize;

/// The product page of the book to extract.
const BOOK_URL: &str = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

/// What the model is asked to read from the page.
const INSTRUCTION: &str = "From this book's product page, return the book it describes. \
    title: the book's title as text. \
    price: its price in pounds as a decimal number, such as 51.77, without the pound sign. \
    stock: the number of copies in stock, as an integer, from the availability line. \
    rating: the star rating as an integer from 1 to 5; the page gives it as a word such as Three. \
    description: the book's description as text. \
    Use only what the page shows, and give each field exactly once.";

/// The book as the program needs it. Its schema is derived from these fields and their doc
/// comments, and the engine checks the model's answer against it.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(crate = "goodfetch_collie::schemars")]
pub struct Book {
    /// The title of the book, as text.
    pub title: String,
    /// The price in pounds as a decimal number, such as 51.77, without the pound sign.
    pub price: f64,
    /// The number of copies in stock, as an integer.
    pub stock: u32,
    /// The star rating, an integer from 1 to 5.
    #[schemars(range(min = 1, max = 5))]
    pub rating: u8,
    /// The description of the book, as text.
    pub description: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    let Some(model_name) = std::env::args().nth(1) else {
        eprintln!("usage: s02_typed_extract <provider/model>, for example ollama/qwen3:8b");
        return ExitCode::FAILURE;
    };

    let collie = match Collie::connect(ConnectOptions::default()).await {
        Ok(collie) => collie,
        Err(error) => {
            eprintln!("could not connect to the engine: {error}");
            return ExitCode::FAILURE;
        }
    };

    let outcome = extract_book(&collie, ModelReference::from(model_name)).await;
    if let Err(error) = collie.close().await {
        eprintln!("could not close the connection: {error}");
    }

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report_failure(&error);
            ExitCode::FAILURE
        }
    }
}

/// Runs one Run that uses `model` for its AI calls, reads the book and prints it.
async fn extract_book(collie: &Collie, model: ModelReference) -> Result<(), Error> {
    let run = collie
        .start()
        .settings(Settings::new().set(MODEL, Some(model)))
        .await?;
    let outcome = read_book(&run).await;
    run.close().await?;

    let step = outcome?;
    print_book(&step);
    Ok(())
}

/// Opens the page and asks the model for the book on it.
async fn read_book(run: &Run) -> Result<StepResult<Book>, Error> {
    let page = run.new_page().await?;
    page.goto(BOOK_URL).await?;
    page.extract::<Book>(INSTRUCTION, Schema::<Book>::of())
        .mode(ExtractMode::Model)
        .await
}

/// Prints the book's fields, then the tokens used and their cost.
fn print_book(step: &StepResult<Book>) {
    let book = &step.output;
    println!("title: {}", book.title);
    println!("price: £{:.2}", book.price);
    println!("in stock: {}", book.stock);
    println!("rating: {} of 5", book.rating);
    println!("description: {}", book.description);
    print_usage(&step.usage);
}

/// Prints the tokens and calls a model used, and their price when the price table has one.
fn print_usage(usage: &ModelUsage) {
    println!(
        "tokens: {} input, {} output, {} total, in {} model call(s)",
        usage.input_tokens, usage.output_tokens, usage.tokens, usage.calls
    );
    match &usage.price {
        Some(price) => println!("cost: {} {} (list-price estimate)", price.amount(), price.currency()),
        None => println!("cost: not priced (the price table has no price for this model)"),
    }
}

/// Says which field could not be found or has the wrong type, then any tokens spent before the
/// failure. Other failures are printed as they are.
fn report_failure(error: &Error) {
    if let Some(incomplete) = error.details::<ExtractionIncomplete>() {
        for missing in &incomplete.missing {
            eprintln!("field not found: {} ({})", render_path(&missing.path), missing.reason);
        }
    } else if let Some(invalid) = error.details::<ExtractionInvalid>() {
        eprintln!(
            "the answer from {} after {} attempt(s) has fields of the wrong type:",
            invalid.model, invalid.attempts
        );
        for issue in &invalid.issues {
            eprintln!("wrong type: {}: {}", render_path(&issue.path), issue.message);
        }
    } else {
        eprintln!("extraction failed: {error}");
    }

    if let Some(usage) = error.usage() {
        print_usage(usage);
    }
}

/// A field path as a person reads it, such as `rating` or `items[2].price`.
fn render_path(path: &[PathSegment]) -> String {
    if path.is_empty() {
        return "the whole book".to_owned();
    }
    let mut rendered = String::new();
    for segment in path {
        match segment {
            PathSegment::Field(name) => {
                if !rendered.is_empty() {
                    rendered.push('.');
                }
                rendered.push_str(name);
            }
            PathSegment::Index(index) => rendered.push_str(&format!("[{index}]")),
        }
    }
    rendered
}
