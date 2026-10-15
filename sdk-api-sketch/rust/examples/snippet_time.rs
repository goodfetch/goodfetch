//! Snippet for Time, waits, deadlines, cancellation and Limits (#467).

use std::time::Duration;

use goodfetch_collie::{AriaRole, ConnectOptions, Count, Error, Collie, Limits, LoadState, Money, SelectOption};

async fn search_quotes() -> Result<usize, Error> {
    // Limits on this connection's work: a program may set or lower them, never raise them.
    let options = ConnectOptions::default().limits(Limits::new().price(Money::new("2.00", "EUR")?));
    let collie = Collie::connect(options).await?;

    // The whole Run may last two hours; past that it stops with a Checkpoint and can be resumed.
    let run = collie
        .start()
        .timeout(Duration::from_secs(2 * 60 * 60))
        .limits(Limits::new().model_tokens(200_000))
        .await?;
    let page = run.new_page().await?;
    page.goto("https://quotes.toscrape.com/search.aspx").timeout(Duration::from_secs(10)).await?;

    // Choosing an author posts the form to filter.aspx and reloads the tag list: `during` arms the waiter before driving the action (P02).
    let (tags, _selected) = page
        .wait_for_response("**/filter.aspx")
        .during(page.locator("#author").select_option([SelectOption::from("Albert Einstein")]))
        .await?;
    let _ = tags.status();

    // The runtime's own deadline: on expiry the call is dropped, which sends `$/cancel`.
    let search = page
        .wait_for_response("**/filter.aspx")
        .during(page.get_by_role(AriaRole::Button.name("Search")).click());
    let _ = tokio_like_timeout(Duration::from_secs(5), search).await;

    let count = page.locator(".results .quote").wait_for_count(Count::at_least(1)).await?;
    page.wait_for_load_state(LoadState::NetworkIdle).no_timeout().await?; // explicitly no timeout

    collie.close().await?;
    Ok(count)
}

/// Stands in for `tokio::time::timeout`, which the sketch does not depend on.
async fn tokio_like_timeout<F: Future>(limit: Duration, future: F) -> Option<F::Output> {
    let _ = limit;
    Some(future.await)
}

fn main() {
    let _ = search_quotes();
}
