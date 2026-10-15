//! The existing program. Rust has no Playwright: this program launches Chromium itself (the
//! executable is its first argument) with a remote debugging port, 127.0.0.1:9222, and drives
//! that browser's page through the SDK: it opens https://books.toscrape.com/, clicks the "Travel"
//! category, clicks the first book and prints its title, then closes the browser itself.
//!
//! Added: right after the "Travel" click, a language model extracts the title and price of every
//! book listed on that page. The step only reads the page; if it fails, the failure is reported and
//! the program carries on with its page and still closes its browser.

use std::process::Command;
use std::time::Duration;

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::{
    AriaRole, Collie, ConnectOptions, EngineChoice, Error, Page, Schema, SessionSource,
};
use serde::Deserialize;

/// One book of a listing.
#[derive(Debug, Deserialize, JsonSchema)]
struct Book {
    /// The book's full title.
    title: String,
    /// The price as the page shows it, currency sign included.
    price: String,
}

/// Every book listed on the page.
#[derive(Debug, Deserialize, JsonSchema)]
struct Listing {
    /// All the books in the listing.
    books: Vec<Book>,
}

/// The added step: a model reads the page and returns every listed book's title and price. It
/// navigates, closes and clicks nothing, and leaves the page as it found it.
async fn extract_listing(page: &Page) -> Result<Listing, Error> {
    let result = page
        .extract(
            "the title and price of every book listed on this page",
            Schema::<Listing>::of(),
        )
        .model("vercel/openai/gpt-5-mini")
        .timeout(Duration::from_secs(120))
        .await?;
    Ok(result.output)
}

/// The existing page steps, driven through the SDK on the launched browser.
async fn drive() -> Result<(), Box<dyn std::error::Error>> {
    let collie = Collie::connect(ConnectOptions::default().engine(EngineChoice::Linked)).await?;
    let outcome = drive_with(&collie).await;
    // Closing the connection never closes the borrowed browser; the program does that itself.
    if let Err(error) = collie.close().await {
        eprintln!("closing the Collie connection failed: {error}");
    }
    outcome
}

async fn drive_with(collie: &Collie) -> Result<(), Box<dyn std::error::Error>> {
    let run = collie.start().await?;
    let outcome = drive_page(&run).await;
    if let Err(error) = run.close().await {
        eprintln!("closing the run failed: {error}");
    }
    outcome
}

async fn drive_page(run: &goodfetch_collie::Run) -> Result<(), Box<dyn std::error::Error>> {
    let source = SessionSource::CdpUrl {
        url: "http://127.0.0.1:9222".to_owned(),
        headers: Vec::new(),
        domains: vec!["toscrape.com".to_owned()],
        exclusive: true,
    };

    // The browser needs a moment before its debugging port answers.
    let mut attempt = 0;
    let session = loop {
        match run.new_session().source(source.clone()).await {
            Ok(session) => break session,
            Err(error) if attempt < 20 => {
                attempt += 1;
                eprintln!("waiting for the browser's debugging port: {error}");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(error) => return Err(error.into()),
        }
    };

    let page = match session.pages().into_iter().next() {
        Some(page) => page,
        None => session.new_page().await?,
    };

    page.goto("https://books.toscrape.com/").await?;
    page.get_by_role(AriaRole::Link.name("Travel").exact()).click().await?;

    // The added step: it must never stop the program.
    match extract_listing(&page).await {
        Ok(listing) => {
            for book in &listing.books {
                println!("{} - {}", book.title, book.price);
            }
        }
        Err(error) => eprintln!("extracting the book listing failed, carrying on: {error}"),
    }

    page.locator("article.product_pod h3 a").first().click().await?;
    let title = page.locator("h1").text_content().await?;
    println!("{title}");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chromium = std::env::args().nth(1).unwrap_or_else(|| "chromium".to_owned());
    let mut browser = Command::new(chromium)
        .args(["--headless=new", "--remote-debugging-port=9222"])
        .spawn()?;

    let outcome = drive().await;

    browser.kill()?;
    browser.wait()?;
    outcome
}
