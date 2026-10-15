//! Extracts one book from books.toscrape.com as a typed object, read by a language model, and
//! prints the tokens used and their cost next to the result.
//!
//! Usage: `s02_typed_extract <model reference>`, such as `anthropic/claude-sonnet-4-5` or the
//! local `ollama/qwen3:8b`. A local model has no price in the price table, so its cost is shown as
//! not priced.

use std::process::ExitCode;

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::settings::ai;
use goodfetch_collie::{
    Collie, ConnectOptions, Error, ExtractMode, ExtractionIncomplete, ExtractionInvalid,
    ModelReference, ModelUsage, PathSegment, Schema, Settings,
};
use serde::{Deserialize, Deserializer, Serialize};

const URL: &str = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

const INSTRUCTION: &str = "Read the book shown on this page. Give its title, its price in pounds \
as a plain decimal number without the currency sign, the number of copies in stock as an integer \
(from the availability line), its star rating as an integer from 1 to 5, and its description \
text (the product description paragraph). Leave nothing out and invent nothing: if a field is \
not on the page, say it is absent.";

/// The book on the page.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Book {
    /// The book's title.
    title: String,
    /// The price in pounds sterling as a decimal number, such as 51.77, without the currency sign.
    #[serde(deserialize_with = "price_in_pounds")]
    price_gbp: f64,
    /// The number of copies in stock, as an integer.
    copies_in_stock: u32,
    /// The star rating, an integer from 1 to 5.
    #[serde(deserialize_with = "star_rating")]
    #[schemars(range(min = 1, max = 5))]
    star_rating: u8,
    /// The book's description text.
    description: String,
}

fn price_in_pounds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let price = f64::deserialize(deserializer)?;
    if price.is_finite() && price >= 0.0 {
        Ok(price)
    } else {
        Err(serde::de::Error::custom(format!(
            "price_gbp must be a non-negative number of pounds, got {price}"
        )))
    }
}

fn star_rating<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let rating = u8::deserialize(deserializer)?;
    if (1..=5).contains(&rating) {
        Ok(rating)
    } else {
        Err(serde::de::Error::custom(format!(
            "star_rating must be one of 1 to 5, got {rating}"
        )))
    }
}

/// A failure, with the model spend that came before it when there was any.
struct Failure {
    message: String,
    usage: Option<ModelUsage>,
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self { message: describe(&error), usage: error.usage().cloned() }
    }
}

fn path_text(path: &[PathSegment]) -> String {
    if path.is_empty() {
        return "(the whole object)".to_owned();
    }
    let mut text = String::new();
    for segment in path {
        match segment {
            PathSegment::Field(name) => {
                if !text.is_empty() {
                    text.push('.');
                }
                text.push_str(name);
            }
            PathSegment::Index(index) => text.push_str(&format!("[{index}]")),
        }
    }
    text
}

/// Says what failed, and for an extraction, which field.
fn describe(error: &Error) -> String {
    let mut text = format!("{} in {}: {}", error.code().as_str(), error.operation(), error.message());
    if let Some(incomplete) = error.details::<ExtractionIncomplete>() {
        for missing in &incomplete.missing {
            text.push_str(&format!(
                "\n  field `{}` was not found on the page ({})",
                path_text(&missing.path),
                missing.reason
            ));
        }
    }
    if let Some(invalid) = error.details::<ExtractionInvalid>() {
        for issue in &invalid.issues {
            text.push_str(&format!(
                "\n  field `{}` does not have the expected type or value: {}",
                path_text(&issue.path),
                issue.message
            ));
        }
    }
    if let Some(fix) = error.fix() {
        text.push_str(&format!("\n  fix: {fix}"));
    }
    text
}

/// Checks what the types cannot: text fields must hold something.
fn check(book: Book) -> Result<Book, Failure> {
    for (field, value) in [("title", &book.title), ("description", &book.description)] {
        if value.trim().is_empty() {
            return Err(Failure {
                message: format!("field `{field}` is empty: it was not found on the page"),
                usage: None,
            });
        }
    }
    Ok(book)
}

async fn read_book(collie: &Collie, model: &str) -> Result<(Book, ModelUsage), Failure> {
    let settings = Settings::new().set(ai::MODEL, ModelReference::from(model));
    let run = collie.start().settings(settings).await?;
    let outcome = async {
        let page = run.new_page().await?;
        page.goto(URL).await?;
        let step = page
            .extract(INSTRUCTION, Schema::<Book>::of())
            .mode(ExtractMode::Model)
            .await?;
        Ok::<_, Error>((step.output, step.usage))
    }
    .await;
    let _ = run.close().await;
    let (book, usage) = outcome?;
    match check(book) {
        Ok(book) => Ok((book, usage)),
        Err(mut failure) => {
            failure.usage = Some(usage);
            Err(failure)
        }
    }
}

fn print_usage(usage: &ModelUsage) {
    println!("model calls:   {}", usage.calls);
    println!("input tokens:  {}", usage.input_tokens);
    println!("output tokens: {}", usage.output_tokens);
    println!("total tokens:  {}", usage.tokens);
    match &usage.price {
        Some(price) => println!("cost:          {} {}", price.amount(), price.currency()),
        None => println!("cost:          not priced (no price in the price table, as for a local model)"),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let Some(model) = std::env::args().nth(1) else {
        eprintln!("usage: s02_typed_extract <model>, such as anthropic/claude-sonnet-4-5 or ollama/qwen3:8b");
        return ExitCode::from(2);
    };

    let collie = match Collie::connect(ConnectOptions::default()).await {
        Ok(collie) => collie,
        Err(error) => {
            eprintln!("could not connect: {}", describe(&error));
            return ExitCode::FAILURE;
        }
    };
    let outcome = read_book(&collie, &model).await;
    let _ = collie.close().await;

    match outcome {
        Ok((book, usage)) => {
            match serde_json::to_string_pretty(&book) {
                Ok(json) => println!("{json}"),
                Err(error) => println!("{book:?} ({error})"),
            }
            print_usage(&usage);
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("extraction failed: {}", failure.message);
            if let Some(usage) = &failure.usage {
                eprintln!("spent before failing:");
                print_usage(usage);
            }
            ExitCode::FAILURE
        }
    }
}
