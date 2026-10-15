//! The existing program. Rust has no Playwright: this program launches Chromium itself (the
//! executable is its first argument) with a remote debugging port, 127.0.0.1:9222, and drives
//! that browser's page through the SDK: it opens https://books.toscrape.com/, clicks the "Travel"
//! category, clicks the first book and prints its title, then closes the browser itself.

use std::process::Command;
use collie_client::page::Page;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chromium = std::env::args().nth(1).unwrap_or_else(|| "chromium".to_owned());
    let mut browser = Command::new(chromium)
        .args(["--headless=new", "--remote-debugging-port=9222"])
        .spawn()?

    // Connect to the browser
    let browser = collie_client::connect("http://127.0.0.1:9222").await?;
    let context = browser.new_context().await?;
    let page = context.new_page().await?;

    // Open the page
    page.goto("https://books.toscrape.com/").await?;

    // Click the "Travel" category
    page.click("text=Travel").await?;

    // Extract the title and price of every book listed
    let books = page.eval_on_selector_all(".product_pod", r#"
        Array.from(document.querySelectorAll(".product_pod"), el => {
            const title = el.querySelector("h3 a").innerText;
            const price = el.querySelector(".price_color").innerText;
            return { title, price };
        })
    "#).await?;
    for book in books {
        println!("Title: {}, Price: {}", book["title"], book["price"]);
    }

    // Click the first book
    page.click(".product_pod:first-child h3 a").await?;

    // Print the title of the first book
    let title = page.eval_on_selector("h1", r#"
        document.querySelector("h1").innerText
    "#).await?;
    println!("Title of the first book: {}", title);

    // Close the browser
    browser.close().await?;

    Ok(())
}