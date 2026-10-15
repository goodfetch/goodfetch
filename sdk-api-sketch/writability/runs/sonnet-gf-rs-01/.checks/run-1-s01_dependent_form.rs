//! Opens quotes.toscrape.com/search.aspx, chooses an author, then one of the tags the page then
//! offers, presses Search and returns every quote shown as typed objects.

use std::fmt;
use std::process::ExitCode;

use goodfetch_collie::{
    AriaRole, Collie, ConnectOptions, ElementState, Error, Filter, LoadState, Page, Run,
    SelectOption, TextMatch,
};
use serde::{Deserialize, Serialize};

const URL: &str = "https://quotes.toscrape.com/search.aspx";
const AUTHOR: &str = "Albert Einstein";
const TAG: &str = "inspirational";

/// One quote of the results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    /// The quote's text.
    pub text: String,
    /// Its author.
    pub author: String,
    /// The tag it was found under.
    pub tag: String,
}

/// A quote as the page shows it, before the tag is checked.
#[derive(Debug, Deserialize)]
struct ShownQuote {
    text: String,
    author: String,
    tag: Option<String>,
}

/// A failure of the task: which step, and what was seen.
#[derive(Debug)]
pub struct TaskError {
    /// The step that failed.
    pub step: String,
    /// What was seen there.
    pub seen: String,
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "step \"{}\" failed: {}", self.step, self.seen)
    }
}

impl std::error::Error for TaskError {}

impl TaskError {
    fn new(step: &str, seen: impl Into<String>) -> Self {
        Self { step: step.to_owned(), seen: seen.into() }
    }
}

/// Turns a Goodfetch error into a [`TaskError`] naming the step.
fn at(step: &'static str) -> impl FnOnce(Error) -> TaskError {
    move |error| {
        let mut seen = format!("{} ({})", error.message(), error.code().as_str());
        if let Some(observed) = error.observed() {
            seen.push_str(&format!("; observed: {observed}"));
        }
        if let Some(url) = error.url() {
            seen.push_str(&format!("; at {url}"));
        }
        TaskError::new(step, seen)
    }
}

/// Drives the page through the form and reads the results.
async fn search(page: &Page) -> Result<Vec<Quote>, TaskError> {
    // Step 1: open the page.
    let response = page.goto(URL).await.map_err(at("open the search page"))?;
    if let Some(response) = response {
        if !response.ok() {
            return Err(TaskError::new(
                "open the search page",
                format!("{} answered {} {}", response.url(), response.status(), response.status_text()),
            ));
        }
    }

    // Step 2: choose the author; the page reloads and the tag list then offers that author's tags.
    let author_select = page.locator("#author");
    let chosen = author_select
        .select_option([SelectOption::Label(AUTHOR.to_owned())])
        .await
        .map_err(at("choose the author"))?;
    if chosen.is_empty() {
        return Err(TaskError::new("choose the author", format!("no option \"{AUTHOR}\" was selected")));
    }

    // Step 3: wait for the tag list to offer the tag, then choose it.
    let tag_options = page.locator("#tag option");
    let wanted_tag = tag_options.filter(Filter::new().has_text(TextMatch::exact(TAG)));
    if let Err(error) = wanted_tag.wait_for(ElementState::Attached).await {
        let offered = tag_options.all_text_contents().await.unwrap_or_default();
        return Err(TaskError::new(
            "wait for the tag list of the author",
            format!("the tag list did not offer \"{TAG}\" ({error}); it offered {offered:?}"),
        ));
    }
    let chosen = page
        .locator("#tag")
        .select_option([SelectOption::Label(TAG.to_owned())])
        .await
        .map_err(at("choose the tag"))?;
    if chosen.is_empty() {
        return Err(TaskError::new("choose the tag", format!("no option \"{TAG}\" was selected")));
    }

    // Step 4: press Search and wait for the results to load.
    let search_button = page.get_by_role(AriaRole::Button.name("Search"));
    let (_, ()) = page
        .wait_for_response("**/filter.aspx")
        .during(search_button.click())
        .await
        .map_err(at("press Search"))?;
    page.wait_for_load_state(LoadState::Load).await.map_err(at("wait for the results"))?;

    // Step 5: read the quotes shown.
    let quotes = page.locator(".quote");
    let shown = quotes.count().await.map_err(at("count the quotes"))?;
    if shown == 0 {
        return Ok(Vec::new());
    }
    let read: Vec<ShownQuote> = quotes
        .evaluate_all(
            "elements => elements.map(element => ({
                text: (element.querySelector('.content')?.textContent ?? '').trim(),
                author: (element.querySelector('.author')?.textContent ?? '').trim(),
                tag: element.querySelector('.tag')?.textContent?.trim() ?? null,
            }))",
        )
        .await
        .map_err(at("read the quotes"))?;

    let mut result = Vec::with_capacity(read.len());
    for item in read {
        let tag = item.tag.unwrap_or_else(|| TAG.to_owned());
        if item.text.is_empty() || item.author.is_empty() {
            return Err(TaskError::new(
                "read the quotes",
                format!("a quote lacked its text or author: text {:?}, author {:?}", item.text, item.author),
            ));
        }
        if item.author != AUTHOR || tag != TAG {
            return Err(TaskError::new(
                "check the results",
                format!("expected \"{AUTHOR}\" under \"{TAG}\" but saw a quote by {:?} under {tag:?}", item.author),
            ));
        }
        result.push(Quote { text: item.text, author: item.author, tag });
    }
    Ok(result)
}

/// Opens a page in the Run, searches, and closes the page on every path.
async fn in_page(run: &Run) -> Result<Vec<Quote>, TaskError> {
    let page = run.new_page().await.map_err(at("open a page"))?;
    let result = search(&page).await;
    let closed = page.close().await.map_err(at("close the page"));
    let quotes = result?;
    closed?;
    Ok(quotes)
}

/// Starts a Run, does the task in it, and ends the Run on every path.
async fn in_run(collie: &Collie) -> Result<Vec<Quote>, TaskError> {
    let run = collie.start().await.map_err(at("start a run"))?;
    let result = in_page(&run).await;
    let ended = match &result {
        Ok(_) => run.finish().await.map(|_| ()).map_err(at("finish the run")),
        Err(_) => run.close().await.map_err(at("close the run")),
    };
    let quotes = result?;
    ended?;
    Ok(quotes)
}

/// Connects, does the task, and closes the connection on every path.
async fn run_task() -> Result<Vec<Quote>, TaskError> {
    let collie = Collie::connect(ConnectOptions::default()).await.map_err(at("connect"))?;
    let result = in_run(&collie).await;
    let closed = collie.close().await.map_err(at("close the connection"));
    let quotes = result?;
    closed?;
    Ok(quotes)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run_task().await {
        Ok(quotes) => {
            if quotes.is_empty() {
                println!("No quote matches \"{AUTHOR}\" with the tag \"{TAG}\".");
            }
            match serde_json::to_string_pretty(&quotes) {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("could not write the quotes: {error}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
