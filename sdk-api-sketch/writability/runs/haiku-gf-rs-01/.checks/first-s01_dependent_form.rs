//! Dependent form: on quotes.toscrape.com's search page, choose the author "Albert Einstein" (the
//! page reloads and its tag list then offers that author's tags), choose the tag "inspirational",
//! press Search, and print every quote shown as a typed object. A page that does not behave as the
//! task describes fails with the step and what was seen. The connection, the Run and its page are
//! all closed before the program ends.

use std::fmt;
use std::process::ExitCode;

use goodfetch_collie::Error as CollieError;
use goodfetch_collie::{
    AriaRole, Collie, ConnectOptions, Count, ErrorCode, Filter, Locator, Page, Run, SelectOption,
    TextMatch, UrlMatcher,
};

const SEARCH_URL: &str = "https://quotes.toscrape.com/search.aspx";
const AUTHOR: &str = "Albert Einstein";
const TAG: &str = "inspirational";

const AUTHOR_SELECT: &str = "#author";
const TAG_SELECT: &str = "#tag";
const TAG_OPTIONS: &str = "#tag option";
const QUOTE_CARD: &str = "div.quote";
const QUOTE_TEXT: &str = "span.text";
const QUOTE_AUTHOR: &str = "small.author";

/// One quote shown by the search results: its text, its author and the tag the search chose.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Quote {
    /// The quote's text, as shown on the page.
    pub text: String,
    /// The author the quote names.
    pub author: String,
    /// The tag the search was run with.
    pub tag: String,
}

/// Why the program failed: the step that failed and, for a page that did not behave as described,
/// what it showed instead.
#[derive(Debug)]
enum ScrapeError {
    /// A Collie call failed at `step`.
    Call {
        step: &'static str,
        error: CollieError,
    },
    /// The page at `step` was not as the task describes; `seen` says what it showed.
    Unexpected { step: &'static str, seen: String },
}

impl fmt::Display for ScrapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call { step, error } => {
                write!(f, "step {step:?} failed with {:?}: {error}", error.code())
            }
            Self::Unexpected { step, seen } => write!(f, "step {step:?} failed, saw {seen}"),
        }
    }
}

fn at<T>(step: &'static str, result: Result<T, CollieError>) -> Result<T, ScrapeError> {
    result.map_err(|error| ScrapeError::Call { step, error })
}

fn page_response(path: &'static str) -> UrlMatcher {
    UrlMatcher::Predicate(Box::new(move |url: &str| url.contains(path)))
}

#[tokio::main]
async fn main() -> ExitCode {
    // The outer error is the connection or its close failing; the inner one is the search's.
    match scrape().await {
        Ok(Ok(quotes)) => print_quotes(&quotes),
        Ok(Err(error)) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: the Collie connection failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_quotes(quotes: &[Quote]) -> ExitCode {
    if quotes.is_empty() {
        eprintln!("No quote matches author {AUTHOR:?} with tag {TAG:?}.");
    }
    match serde_json::to_string_pretty(quotes) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: could not encode the quotes: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the search on a connection that `Collie::scope` closes on every return and error.
async fn scrape() -> Result<Result<Vec<Quote>, ScrapeError>, CollieError> {
    Collie::scope(ConnectOptions::default(), async |collie| {
        Ok(search(collie).await)
    })
    .await
}

/// Starts a Run, searches in it, then ends it: finished on success, stopped and resumable on failure.
async fn search(collie: &Collie) -> Result<Vec<Quote>, ScrapeError> {
    let run = at("start a run", collie.start().await)?;
    let outcome = search_in_run(&run).await;
    let ended = match &outcome {
        Ok(_) => at("finish the run", run.finish().await).map(|_| ()),
        Err(_) => Ok(()),
    };
    let closed = at("close the run", run.close().await);
    let quotes = outcome?;
    ended?;
    closed?;
    Ok(quotes)
}

async fn search_in_run(run: &Run) -> Result<Vec<Quote>, ScrapeError> {
    let page = at("open a page", run.new_page().await)?;
    let outcome = search_on_page(&page).await;
    let closed = at("close the page", page.close().await);
    let quotes = outcome?;
    closed?;
    Ok(quotes)
}

async fn search_on_page(page: &Page) -> Result<Vec<Quote>, ScrapeError> {
    open_search_page(page).await?;
    choose_author(page).await?;
    choose_tag(page).await?;
    press_search(page).await?;
    read_quotes(page).await
}

async fn open_search_page(page: &Page) -> Result<(), ScrapeError> {
    let step = "open the search page";
    match at(step, page.goto(SEARCH_URL).await)? {
        Some(response) if !response.ok() => Err(ScrapeError::Unexpected {
            step,
            seen: format!("HTTP status {}", response.status()),
        }),
        _ => Ok(()),
    }
}

/// Choosing an author reloads the page; the reload is awaited as part of the same step, armed
/// around the selection so it cannot be missed.
async fn choose_author(page: &Page) -> Result<(), ScrapeError> {
    let step = "choose the author";
    let selection = page
        .locator(AUTHOR_SELECT)
        .select_option([SelectOption::Label(AUTHOR.to_owned())]);
    let reloaded = page
        .wait_for_response(page_response("search.aspx"))
        .during(selection)
        .await;
    at(step, reloaded).map(|_| ())
}

/// After the reload, the tag list must offer the tag the task chooses.
async fn choose_tag(page: &Page) -> Result<(), ScrapeError> {
    let step = "check the tag list";
    let tags = page.locator(TAG_OPTIONS);
    let offered = tags.filter(Filter::new().has_text(TextMatch::exact(TAG)));
    match offered.wait_for_count(Count::at_least(1)).await {
        Ok(_) => {}
        Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED => {
            let seen = tags
                .all_text_contents()
                .await
                .map(|texts| format!("the tags [{}]", texts.join(", ")))
                .unwrap_or_else(|_| String::from("an unreadable tag list"));
            return Err(ScrapeError::Unexpected {
                step,
                seen: format!("{seen} after choosing {AUTHOR:?}, with no {TAG:?} among them"),
            });
        }
        Err(error) => return Err(ScrapeError::Call { step, error }),
    }
    at(
        "choose the tag",
        page.locator(TAG_SELECT)
            .select_option([SelectOption::Label(TAG.to_owned())])
            .await,
    )
    .and_then(|selected| {
        if selected.is_empty() {
            Err(ScrapeError::Unexpected {
                step: "choose the tag",
                seen: format!("no option selected for {TAG:?}"),
            })
        } else {
            Ok(())
        }
    })
}

async fn press_search(page: &Page) -> Result<(), ScrapeError> {
    let step = "press Search";
    let search = page.get_by_role(AriaRole::Button.name("Search").exact());
    let answered = page
        .wait_for_response(page_response("search.aspx"))
        .during(search.click())
        .await;
    at(step, answered).map(|_| ())
}

/// The quotes shown. Zero quotes is an empty result only when the page carries its own
/// "No quotes found" notice; otherwise the page did not behave as described.
async fn read_quotes(page: &Page) -> Result<Vec<Quote>, ScrapeError> {
    let step = "read the quotes";
    let cards = page.locator(QUOTE_CARD);
    match cards.wait_for_count(Count::at_least(1)).await {
        Ok(_) => {}
        Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED => {
            let notice = TextMatch::regex("(?i)no quotes found");
            if at(step, page.get_by_text(notice).is_visible().await)? {
                return Ok(Vec::new());
            }
            let shown = page_text(page).await;
            return Err(ScrapeError::Unexpected {
                step,
                seen: format!("no quote and no \"No quotes found\" notice, the page shows {shown:?}"),
            });
        }
        Err(error) => return Err(ScrapeError::Call { step, error }),
    }
    let mut quotes = Vec::new();
    for card in at(step, cards.all().await)? {
        quotes.push(read_card(&card).await?);
    }
    Ok(quotes)
}

async fn read_card(card: &Locator) -> Result<Quote, ScrapeError> {
    let step = "read a quote";
    let text = at(step, card.locator(QUOTE_TEXT).text_content().await)?;
    let author = at(step, card.locator(QUOTE_AUTHOR).text_content().await)?;
    let author = author.trim().to_owned();
    if author != AUTHOR {
        return Err(ScrapeError::Unexpected {
            step,
            seen: format!("a quote by {author:?} where the search was for {AUTHOR:?}"),
        });
    }
    Ok(Quote {
        text: text.trim().to_owned(),
        author,
        tag: TAG.to_owned(),
    })
}

/// The start of the page's body text, for an error message.
async fn page_text(page: &Page) -> String {
    match page.locator("body").inner_text().await {
        Ok(text) => text.chars().take(200).collect(),
        Err(error) => format!("an unreadable page ({error})"),
    }
}
