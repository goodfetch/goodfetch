//! Snippet for Collie's page API (#472, ADR 0085).

use std::future::IntoFuture;
use std::pin::pin;
use std::time::Duration;

use futures_util::TryStreamExt;
use futures_util::future::{Either, select, try_join_all};
use goodfetch_sdk_sketch::{
    AbortReason, AriaRole, ConnectOptions, DialogKind, ElementState, Error, ErrorCode, Goodfetch,
    Locator, LocatorHandlerOptions, Page, ResourceType, SelectOption, events,
};
use serde_json::{Value, json};

/// A quote, as golden scenario 01 returns it.
#[derive(Debug)]
pub struct Quote {
    /// Its text.
    pub text: String,
    /// Its author.
    pub author: String,
    /// The tag it was found under.
    pub tag: String,
}

/// Golden scenario 01, "Fill a dependent form and read the results".
async fn einstein_quotes() -> Result<Vec<Quote>, Error> {
    let gf = Goodfetch::connect(ConnectOptions::default()).await?;
    let run = gf.collie().start().await?;
    let page = run.new_page().await?;

    page.goto("https://quotes.toscrape.com/search.aspx").await?;
    // Choosing an author posts the form back; the action waits for that navigation to commit, and
    // the tag's option is waited for by the guard like any other condition (K01).
    page.get_by_label("Author")
        .select_option([SelectOption::from("Albert Einstein")])
        .await?;
    page.get_by_label("Tag")
        .select_option([SelectOption::from("inspirational")])
        .await?;
    page.wait_for_response("**/filter.aspx")
        .during(page.get_by_role(AriaRole::Button.name("Search")).click())
        .await?;

    // An empty list is an answer, not an error (G1). Reads run concurrently on the page, each in
    // one round trip; gestures would run one at a time.
    let read = async |quote: Locator| -> Result<Quote, Error> {
        Ok(Quote {
            text: quote.locator(".content").text_content().await?,
            author: quote.locator(".author").text_content().await?,
            tag: quote.locator(".tag").text_content().await?,
        })
    };
    let quotes = page.locator(".results .quote").all().await?;
    let found = try_join_all(quotes.into_iter().map(read)).await?;
    gf.close().await?;
    Ok(found)
}

/// Strictness: an ambiguous target is waited out like any other condition, then fails with every
/// match.
async fn click_next(page: &Page) -> Result<(), Error> {
    let next = page.get_by_role(AriaRole::Link.name("Next"));
    match next.click().timeout(Duration::from_secs(5)).await {
        Err(err) if err.code() == &ErrorCode::TargetNotUnique => next.first().click().await, // the visible opt-out
        other => other,
    }
}

/// Routes and dialogs arrive as streams the program drives; an item dropped unanswered falls
/// through to the default (golden scenario 06's browser part).
async fn last_scrolling_page(page: &Page) -> Result<Option<Value>, Error> {
    let answer = async {
        let media = page
            .route("**/*")
            .try_for_each_concurrent(16, async |route| match route.request().resource_type() {
                ResourceType::Image | ResourceType::Font => {
                    route.abort().reason(AbortReason::BlockedByClient).await
                }
                _ => route.fallback().await,
            });
        let dialogs = page.events(events::Dialog).try_for_each(async |dialog| {
            if dialog.kind() == DialogKind::Confirm && dialog.message().starts_with("Are you sure")
            {
                dialog.accept().await?;
            }
            Ok(()) // dropped otherwise: the default, a confirm dismissed (ADR 0014)
        });
        futures_util::try_join!(media, dialogs).map(|_| ())
    };
    let scroll = async {
        page.goto("https://quotes.toscrape.com/scroll").await?;
        // Input with no target is marked unguarded; the finished request's body is kept.
        let (last, ()) = page
            .wait_for_request_finished("**/api/quotes?page=*")
            .during(page.mouse().wheel(0.0, 10_000.0))
            .await?;
        match last.response().await? {
            Some(response) if response.ok() => Ok(Some(response.json().await?)),
            _ => Ok(None),
        }
    };
    // Once the scroll ends, the answering future is dropped, and with it the subscriptions.
    match select(pin!(scroll), pin!(answer)).await {
        Either::Left((json, _)) => json,
        Either::Right((answered, _)) => answered.map(|()| None),
    }
}

/// Page functions: JavaScript source, one explicit JSON argument, typed results.
async fn read_stats(page: &Page) -> Result<usize, Error> {
    let long_tags: usize = page
        .evaluate("({ min }) => [...document.querySelectorAll('.tag')].filter((t) => t.textContent.length > min).length")
        .arg(json!({ "min": 3 }))
        .await?;
    let _next: Option<String> = page.locator("li.next > a").get_attribute("href").await?; // `None` when absent
    match page.evaluate::<Value>("() => document.body").await {
        Err(err) if err.code() == &ErrorCode::PageValueInvalid => {} // a DOM node is not JSON
        other => drop(other?),
    }
    Ok(long_tags)
}

/// Frames are reached through locators, same-origin or not; popups and downloads are values.
async fn pay_in_frame(page: &Page) -> Result<(), Error> {
    page.frame_locator("iframe[title='Checkout']")
        .get_by_role(AriaRole::Button.name("Pay"))
        .click()
        .await?;
    page.frame_locator(None)
        .get_by_text("Paid")
        .wait_for(ElementState::Visible)
        .await?; // every frame searched

    let (receipt, ()) = page
        .wait_for_event(events::Popup)
        .during(page.get_by_text("Open receipt").click())
        .await?;
    let (download, ()) = receipt
        .wait_for_event(events::Download)
        .during(receipt.get_by_role(AriaRole::Button.name("PDF")).click())
        .await?;
    download.save_as("receipt.pdf").await?; // streamed to this machine
    receipt.close().await?;

    // Overlays that may appear at any time: the guard yields a turn before each attempt.
    let newsletter = page
        .add_locator_handler(
            &page.get_by_text("Sign up to our newsletter"),
            LocatorHandlerOptions::new(),
        )
        .try_for_each(async |_turn| {
            page.get_by_role(AriaRole::Button.name("No thanks"))
                .click()
                .await
        });
    let orders = page
        .get_by_role(AriaRole::Link.name("Orders"))
        .click()
        .into_future();
    match select(pin!(orders), pin!(newsletter)).await {
        Either::Left((clicked, _)) => clicked,
        Either::Right((handled, _)) => handled,
    }
}

/// The way down: raw CDP for one page, under ADR 0017's rules, Chromium only.
async fn heap_metrics(page: &Page) -> Result<Value, Error> {
    let cdp = page.new_cdp_session().await?;
    cdp.send("Performance.enable", json!({})).await?;
    let metrics = cdp.send("Performance.getMetrics", json!({})).await?;
    cdp.close().await?;
    Ok(metrics)
}

fn main() {
    let _ = (
        einstein_quotes,
        click_next,
        last_scrolling_page,
        read_stats,
        pay_in_frame,
        heap_metrics,
    );
}
