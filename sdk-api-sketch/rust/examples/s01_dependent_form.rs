//! Golden scenario 01, "Fill a dependent form and read the results" (Collie, Rust).

use goodfetch_collie::settings::session;
use goodfetch_collie::{
    AriaRole, Collie, ConnectOptions, ElementState, Error, LoadState, PageCheck, PageMatch,
    SelectOption, Settings,
};

/// A quote the search shows.
#[derive(Debug)]
pub struct Quote {
    /// Its text.
    pub text: String,
    /// Its author.
    pub author: String,
    /// The tag it was found under.
    pub tag: String,
}

/// Every quote the search shows for `author` and `tag`: an empty list when none matches.
async fn search_quotes(author: &str, tag: &str) -> Result<Vec<Quote>, Error> {
    Collie::scope(ConnectOptions::default(), async |collie| {
        // The Page check judges every page: one without the search form (an error page, a changed
        // site) fails the call that reached it with `page_check_failed`, so an empty list is never
        // a broken page.
        let form = PageCheck::new().expected_page(PageMatch::selector("form[name='filterform']"));
        let settings = Settings::new().set(session::PAGE_CHECK, Some(form));
        let run = collie.start().settings(settings).await?;
        let page = run.new_page().await?;

        page.goto("https://quotes.toscrape.com/search.aspx").await?;
        // Choosing the author posts the form back, and the action waits for that navigation to
        // commit.
        page.get_by_label("Author")
            .select_option([SelectOption::from(author)])
            .await?;
        // The guard waits for the reloaded list to offer the tag. Each failure is typed and names
        // the call, its locator and what it saw (here the options offered), with the timeout's
        // setting.
        page.get_by_label("Tag")
            .select_option([SelectOption::from(tag)])
            .await?;
        page.get_by_role(AriaRole::Button.name("Search"))
            .click()
            .await?;
        // The results' document has loaded before it is read.
        page.wait_for_load_state(LoadState::Load).await?;

        // An empty list is an answer, not an error (G1).
        let mut quotes = Vec::new();
        for row in page.locator(".results .quote").all().await? {
            quotes.push(Quote {
                text: row.locator(".content").text_content().await?,
                author: row.locator(".author").text_content().await?,
                tag: row.locator(".tag").text_content().await?,
            });
        }
        if quotes.is_empty() {
            // The site heads its results "Results" only above a match: that heading over no quote
            // is a changed layout.
            page.get_by_role(AriaRole::Heading.name("Results"))
                .wait_for(ElementState::Hidden)
                .await?;
        }
        // Closes the Run's Session and page; `scope` closes the connection on every path. The task
        // reads nothing from the Run report, which is `#[must_use]`.
        let _ = run.finish().await?;
        Ok(quotes)
    })
    .await
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let quotes = search_quotes("Albert Einstein", "inspirational").await?;
    if quotes.is_empty() {
        println!("No quote matches Albert Einstein and inspirational.");
    }
    for quote in &quotes {
        println!("{} ({}, {})", quote.text, quote.author, quote.tag);
    }
    Ok(())
}
