//! Golden scenario 09, "Add an AI step to an existing Playwright program" (ADRs 0056, 0078, 0089,
//! 0091), as Rust has it: Rust has no Playwright, so the existing program launches Chromium itself
//! with a debugging port and drives it through a Session borrowed by its CDP address (ADR 0056's
//! `cdp_url`). The comparison is structural.

use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::{AriaRole, Collie, ConnectOptions, Error, Page, Schema, SessionSource};
use serde::Deserialize;

/// A book the Travel category lists.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Listed {
    /// Its title.
    pub title: String,
    /// Pounds, an exact decimal kept as text.
    #[schemars(regex(pattern = r"^\d+\.\d{2}$"))]
    pub price: String,
}

/// The added step: the model reads the listed books, and a failure is reported, never left to stop
/// the program. The verbs neither navigate nor close the page.
async fn list_books(page: &Page) {
    let listed = page
        .locator("ol.row")
        .extract(
            "every book listed: title and price",
            Schema::<Vec<Listed>>::of(),
        )
        .model("anthropic/claude-sonnet-4-5")
        .await;
    match listed {
        Ok(result) => println!("{:?}", result.output),
        // A typed Error (`model_unavailable`, `extraction_incomplete`, …) with its `fix`.
        Err(error) => eprintln!("The AI step failed; the program carries on: {error}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The existing program launches its own Chromium, given as the first argument.
    let chromium = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "chromium".to_owned());
    let profile = std::env::temp_dir().join("s09-profile");
    let mut browser = Command::new(chromium)
        .args(["--headless=new", "--remote-debugging-port=9222"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .spawn()?;
    while TcpStream::connect("127.0.0.1:9222").is_err() {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic.
    let driven = Collie::scope(ConnectOptions::default(), async |collie| {
        let run = collie.start().await?;
        // A Borrowed browser: closing the Session, the Run or the connection never closes it.
        let session = run
            .new_session()
            .source(SessionSource::CdpUrl {
                url: "http://127.0.0.1:9222".to_owned(),
                headers: Vec::new(),
                domains: Vec::new(),
                exclusive: false,
            })
            .await?;
        let page = session.new_page().await?;
        page.goto("https://books.toscrape.com/").await?;
        page.get_by_role(AriaRole::Link.name("Travel"))
            .click()
            .await?;
        list_books(&page).await;
        page.locator("article.product_pod h3 a")
            .first()
            .click()
            .await?;
        println!("{}", page.locator(".product_main h1").text_content().await?);
        let _ = run.finish().await?;
        Ok::<(), Error>(())
    })
    .await;

    // The existing program closes its own browser, whatever happened.
    browser.kill()?;
    browser.wait()?;
    Ok(driven?)
}
