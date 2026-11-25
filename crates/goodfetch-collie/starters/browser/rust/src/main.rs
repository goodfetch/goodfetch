//! Prints the title of the page its argument names, the Proving ground's
//! when it has none: `cargo run -- https://example.com/`.

use goodfetch_collie::{Collie, ConnectOptions, Error};

/// Opens `url` in a browser page of a new Run on `collie` and answers the
/// page's title.
async fn read_title(collie: &Collie, url: &str) -> Result<String, Error> {
    let run = collie.start().await?;
    let page = run.new_page().await?;
    page.goto(url).await?;
    let title = page.evaluate("() => document.title").await?;
    run.finish().await?;
    Ok(title
        .as_ref()
        .and_then(|title| title.as_str())
        .unwrap_or_default()
        .to_owned())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://proving-ground.goodfetch.dev/".to_owned());
    let title = Collie::scope(ConnectOptions::default(), async |collie| {
        read_title(collie, &url).await
    })
    .await?;
    println!("{title}");
    Ok(())
}

#[cfg(test)]
mod tests;
