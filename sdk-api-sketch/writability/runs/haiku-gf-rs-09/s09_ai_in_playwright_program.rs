//! The existing program. Rust has no Playwright: this program launches Chromium itself (the
//! executable is its first argument) with a remote debugging port, 127.0.0.1:9222, and drives
//! that browser's page through the SDK: it opens https://books.toscrape.com/, clicks the "Travel"
//! category, clicks the first book and prints its title, then closes the browser itself.
//!
//! The added step sits right after the "Travel" click: a language model extracts the title and
//! price of every book listed on that category page. It only reads the page. If it fails, the
//! program reports the failure and carries on with its page as before, then closes its browser.

use std::process::Command;

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::{
    AriaRole, Collie, ConnectOptions, EngineChoice, Error, LoadState, Page, Schema, SessionSource,
};
use serde::Deserialize;

const CDP_URL: &str = "http://127.0.0.1:9222";

/// The model the added step calls: the Vercel AI Gateway's free tier (ADR 0076).
const MODEL: &str = "vercel/anthropic/claude-sonnet-4.5";

/// One book listed on a category page.
#[derive(Debug, Deserialize, JsonSchema)]
struct Book {
    /// The book's title, as the page shows it.
    title: String,
    /// The book's price, as the page shows it, with its currency sign.
    price: String,
}

/// Every book listed on the page, in page order.
#[derive(Debug, Deserialize, JsonSchema)]
struct BookList {
    /// The books on the page.
    books: Vec<Book>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chromium = std::env::args().nth(1).unwrap_or_else(|| "chromium".to_owned());
    let mut browser = Command::new(chromium)
        .args(["--headless=new", "--remote-debugging-port=9222"])
        .spawn()?;

    let outcome = Collie::scope(
        ConnectOptions::default().engine(EngineChoice::Linked),
        async |collie| books_steps(collie).await,
    )
    .await;

    browser.kill()?;
    browser.wait()?;
    outcome?;
    Ok(())
}

/// The page steps, driven through the SDK on the browser this program launched.
async fn books_steps(collie: &Collie) -> Result<(), Error> {
    let run = collie.start().await?;
    let session = run
        .new_session()
        .source(SessionSource::CdpUrl {
            url: CDP_URL.to_owned(),
            headers: Vec::new(),
            domains: vec!["books.toscrape.com".to_owned()],
            exclusive: true,
        })
        .await?;
    let page = session.new_page().await?;

    page.goto("https://books.toscrape.com/").await?;
    page.get_by_role(AriaRole::Link.name("Travel").exact()).click().await?;

    if let Err(error) = list_books(&page).await {
        eprintln!("could not list the books on this page: {error}");
    }

    page.locator("article.product_pod h3 a").first().click().await?;
    let title = page.locator("h1").text_content().await?;
    println!("{title}");
    Ok(())
}

/// The added step: extracts the title and price of every book listed on the current page. It
/// neither navigates nor closes anything; it waits only for the page the click opened to load.
async fn list_books(page: &Page) -> Result<(), Error> {
    page.wait_for_load_state(LoadState::Load).await?;
    let listed = page
        .extract(
            "the title and price of every book listed on this page",
            Schema::<BookList>::of(),
        )
        .model(MODEL)
        .await?;
    for book in listed.output.books {
        println!("{}\t{}", book.price, book.title);
    }
    Ok(())
}
