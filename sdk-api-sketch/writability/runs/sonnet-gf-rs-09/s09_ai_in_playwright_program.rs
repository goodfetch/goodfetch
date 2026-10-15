//! The existing program. Rust has no Playwright: this program launches Chromium itself (the
//! executable is its first argument) with a remote debugging port, 127.0.0.1:9222, and drives
//! that browser's page through the SDK: it opens https://books.toscrape.com/, clicks the "Travel"
//! category, clicks the first book and prints its title, then closes the browser itself.

use std::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chromium = std::env::args().nth(1).unwrap_or_else(|| "chromium".to_owned());
    let mut browser = Command::new(chromium)
        .args(["--headless=new", "--remote-debugging-port=9222"])
        .spawn()?;

    // ... the page steps described above, driven through the SDK on this browser ...

    browser.kill()?;
    browser.wait()?;
    Ok(())
}
