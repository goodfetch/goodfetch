//! Snippet for Collie's AI verbs and Plans (#473, ADR 0091) and the Value transforms (#554, ADR 0123), on golden scenarios 02, 03 and 11.

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::settings::{extract, plan};
use goodfetch_collie::{
    ConnectOptions, Error, ErrorCode, Expected, Collie, Page, Run, Schema, Settings, StepSource,
    expect,
};
use serde::Deserialize;

/// An author, as golden scenario 03 returns it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Author {
    /// The author's name.
    pub name: String,
    /// The birth date, as the page writes it.
    pub birth_date: String,
    /// The birthplace.
    pub birthplace: String,
}

/// Golden scenario 03, "Repeat AI steps with no model call". The first run compiles each step with
/// the model; a later run replays them from quotes.plan.json with no model call, and a step that
/// drifted goes back to the model once, alone.
async fn author_of_first_love_quote() -> Result<Author, Error> {
    let collie = Collie::connect(ConnectOptions::default()).await?;
    let run = collie.start().plan("quotes.plan.json").await?;
    let page = run.new_page().await?;
    page.goto("https://quotes.toscrape.com/").await?;

    for instruction in [
        "open the tag 'love' from the tag list",
        "go to the second page",
        "open the about page of the first quote's author",
    ] {
        let step = page.act(instruction).await?.step;
        if step.source == StepSource::Repair {
            log_repaired(&step.instruction, step.drift.as_ref());
        }
    }

    // A Success check, without a model: the Run is `verified` only if it holds, and only a verified
    // Run promotes a new Plan version (ADR 0009).
    expect(&page)
        .to_have_url(Expected::regex("^https://quotes\\.toscrape\\.com/author/")) // a string must equal the URL
        .await?;
    let author = page
        .extract("the author's name, birth date and birthplace", Schema::<Author>::of())
        .await?
        .output;

    let report = run.finish().await?; // the Run report, with its PlanReport (ADR 0102)
    let _ = report.plan.changed_steps;
    run.close().await?;
    collie.close().await?;
    Ok(author)
}

/// The program's own logging, through `tracing` in a real program.
fn log_repaired(instruction: &str, drift: Option<&goodfetch_collie::Drift>) {
    let _ = (instruction, drift);
}

/// A book, as golden scenario 02 reads it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Book {
    /// Its title.
    pub title: String,
    /// Its price in pounds, kept exact as text.
    pub price: String,
    /// Copies in stock.
    pub in_stock: u32,
    /// One to five stars.
    pub rating: u8,
    /// Its description.
    pub description: String,
}

/// Golden scenario 02, "Extract a typed object with a chosen model": the model chosen for this call
/// alone (a Model reference, ADR 0101), and its usage next to the result. A field not on the page fails, naming it.
async fn typed_book(page: &Page) -> Result<Option<Book>, Error> {
    page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html")
        .await?;
    match page
        .extract("the book on this page", Schema::<Book>::of())
        .model("ollama/qwen3:8b")
        .await
    {
        Ok(result) => {
            let _spent = (result.usage.tokens, result.usage.price);
            Ok(Some(result.output))
        }
        Err(error) if error.code() == ErrorCode::EXTRACTION_INCOMPLETE => {
            if let Some(details) = error.details::<goodfetch_collie::ExtractionIncomplete>() {
                let _ = (&details.missing, &details.found);
            }
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `observe` finds without acting; the program acts on a candidate through its ordinary locator.
async fn search(page: &Page, query: &str) -> Result<(), Error> {
    let boxes = page.observe("the search box in the header").await?.output;
    let Some(search_box) = boxes.first() else {
        return Ok(()); // none is an answer
    };
    search_box.locator.fill(query).await?;
    search_box.locator.press("Enter").await
}

/// The same with `act`, one action per call. Values typed into the page travel as variables: the
/// model and the Plan see only `%query%`, so one Plan step serves every query.
async fn search_with_act(page: &Page, query: &str) -> Result<(), Error> {
    page.act("type %query% into the search box").variable("query", query).await?;
    page.act("submit the search").await?;
    Ok(())
}

/// The program's own model call, outside Collie: the next reference to click, or done.
async fn ask_model(observation: &str) -> (String, bool) {
    let _ = observation;
    unimplemented!()
}

/// A program's own agent loop reads the observation and acts on references.
async fn own_agent_loop(page: &Page) -> Result<(), Error> {
    for _ in 0..10 {
        let observation = page
            .aria_snapshot()
            .mode(goodfetch_collie::AriaSnapshotMode::Ai)
            .await?;
        let (reference, done) = ask_model(&observation).await;
        if done {
            break;
        }
        page.get_by_ref(&reference).click().await?; // references last as long as their element
    }
    Ok(())
}

/// With the Plan frozen, as in CI: no model call, and a drift that cannot be repaired names the step.
async fn frozen_replay(collie: &Collie) -> Result<(), Error> {
    let run = collie
        .start()
        .plan("quotes.plan.json")
        .settings(Settings::new().set(plan::FROZEN, true))
        .await?;
    let page = run.new_page().await?;
    page.goto("https://quotes.toscrape.com/").await?;
    if let Err(error) = page.act("open the tag 'love' from the tag list").await {
        if let Some(drift) = error.details::<goodfetch_collie::PlanDrift>() {
            let _ = (&drift.instruction, &drift.why_no_repair);
        }
        return Err(error);
    }
    expect(&page.get_by_text("Viewing tag: love")).to_be_visible().await
}

/// Values a German shop writes for people, read by Value transforms under its locale (ADR 0123).
/// The engine parses, never the machine's locale; the model picks the text and the transform, never
/// the value, and Grounding runs the transform again.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Offer {
    /// `number` under `de-DE`: "1.099,98" is the exact decimal "1099.98".
    #[schemars(regex(pattern = r"^\d+\.\d{2}$"))]
    pub price: String,
    /// `currency`: "€" becomes "EUR".
    pub currency: String,
    /// `date`: "01.02.2026" becomes "2026-02-01".
    #[schemars(extend("format" = "date"))]
    pub available_from: String,
    /// `relative_time`: "vor 3 Stunden", anchored at the page's fetch time, in RFC 3339.
    #[schemars(extend("format" = "date-time"))]
    pub updated_at: String,
    /// The page's own text, when the program wants it beside the value.
    pub price_text: String,
}

/// Stating the locale and zone is optional: without them the compile chooses the locale from the
/// page (its `lang` is "de"), and a date-time with no offset takes the Session's `browser.timezone_id`.
async fn german_offer(run: &Run) -> Result<Offer, Error> {
    let settings = Settings::new()
        .set(extract::LOCALE, Some("de-DE".into()))
        .set(extract::TIMEZONE_ID, Some("Europe/Berlin".into()));
    let page = run.new_page().settings(settings).await?;
    page.goto("https://shop.example/produkt/42").await?;
    let instruction = "the offer: price, currency, availability date, last update";
    Ok(page.extract(instruction, Schema::<Offer>::of()).await?.output)
}

fn main() {
    let _ = (author_of_first_love_quote, typed_book, search, search_with_act, own_agent_loop, frozen_replay);
    let _ = german_offer;
    let _ = schemars::schema_for!(Author);
}
