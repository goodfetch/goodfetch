//! Finds the cheapest five-star book of the Travel category on books.toscrape.com from one
//! instruction, with Collie's built-in Agent.
//!
//! The Agent is capped at 25 steps and 0.20 USD, stays on books.toscrape.com, and calls only the
//! chosen model's provider (`anthropic`, directly, no gateway). The work is kept in a Plan file, so
//! `--replay` repeats it with no model call at all.

use std::path::Path;

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::settings::plan;
use goodfetch_collie::{
    BoxError, Claim, Collie, ConnectOptions, EngineChoice, Ended, Error, Expected, Limits, Money,
    Page, Schema, Settings, StopReason, Verdict, expect,
};
use serde::Deserialize;

/// The one instruction the program gives.
const TASK: &str = "On this site, find the cheapest book rated five stars in the Travel category \
                    and give its title and price.";

/// The site the Agent starts on and may not leave.
const START_URL: &str = "https://books.toscrape.com/";

/// The model, reached at its own provider.
const MODEL: &str = "anthropic/claude-sonnet-4-5";

/// Where the steps taken are kept, to be repeated without a model.
const PLAN_FILE: &str = "cheapest-travel-book.plan.json";

/// The typed answer.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(crate = "schemars")]
struct CheapestBook {
    /// The book's title, as shown on its page.
    title: String,
    /// The book's price in pounds sterling, without the currency sign.
    price: f64,
}

/// How the work ended.
#[derive(Debug, Clone)]
enum Ending {
    /// A limit, a step cap or a timeout stopped it; the reason is written out.
    StoppedByLimit(String),
    /// The model said it was done, and no condition proved it.
    ReportedDoneByModel,
    /// The program's condition held: a book page of the Travel category with five stars.
    VerifiedByCondition,
    /// It finished, but neither verified nor reported as done.
    Unproven,
}

/// What the program returns.
#[derive(Debug)]
struct Answer {
    /// The title and the price, when the task finished with a complete answer.
    book: Option<CheapestBook>,
    /// How the work ended.
    ending: Ending,
    /// Steps taken.
    step_count: usize,
    /// The steps as plain page API calls, as the engine renders them.
    steps: Vec<String>,
    /// The price of the model calls, in USD.
    spent: Option<String>,
}

/// The Success check: the page reached is a book page in the Travel category with five stars.
async fn is_five_star_travel_book(page: Page) -> Result<(), BoxError> {
    expect(&page)
        .to_have_url(Expected::regex(
            r"^https://books\.toscrape\.com/catalogue/[^/]+/index\.html$",
        ))
        .await?;
    expect(&page.locator("ul.breadcrumb li:nth-child(3)")).to_have_text("Travel").await?;
    expect(&page.locator(".product_main p.star-rating.Five")).to_be_visible().await?;
    Ok::<(), BoxError>(())
}

/// Maps the result's three separate fields to the ending the program reports.
fn ending_of(ended: &Ended, verdict: &Verdict, claimed_done: bool) -> Ending {
    match ended {
        Ended::Stopped(reason) => Ending::StoppedByLimit(match reason {
            StopReason::Limit(name) => format!("limit {name}"),
            other => format!("{other:?}"),
        }),
        Ended::Finished if *verdict == Verdict::Verified => Ending::VerifiedByCondition,
        Ended::Finished if claimed_done => Ending::ReportedDoneByModel,
        Ended::Finished => Ending::Unproven,
    }
}

/// Runs the task once, live with a model or, when `replay` is set, from the Plan alone.
async fn find_cheapest(collie: &Collie, replay: bool) -> Result<Answer, Error> {
    let limits = Limits::new().price(Money::new("0.20", "USD")?);
    let settings = Settings::new().set(plan::FROZEN, replay);
    let run = collie.start().limits(limits).plan(Path::new(PLAN_FILE)).settings(settings).await?;

    let page = run.new_page().await?;
    page.goto(START_URL).await?;

    let mut call = page
        .agent(TASK)
        .schema(Schema::<CheapestBook>::of())
        .success_check(is_five_star_travel_book)
        .max_steps(25)
        .domains(["books.toscrape.com"]);
    if !replay {
        call = call.model(MODEL);
    }
    let result = call.await;

    // Finish the Run whatever the task did: the Plan is written back when the Run ends.
    let report = run.finish().await;
    let result = result?;
    report?;

    let claimed_done = result.reported.as_ref().is_some_and(|r| r.claim == Claim::Done);
    let ending = ending_of(&result.ended, &result.verdict, claimed_done);
    let steps = result
        .steps
        .iter()
        .flat_map(|step| step.actions.iter())
        .filter_map(|action| action.code.clone())
        .collect();
    let spent = result.usage.price.as_ref().map(|m| format!("{} {}", m.amount(), m.currency()));
    Ok(Answer { book: result.output, ending, step_count: result.steps.len(), steps, spent })
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let replay = std::env::args().any(|arg| arg == "--replay");
    // The linked engine only: nothing reaches a Daemon or a remote engine.
    let options = ConnectOptions::default().engine(EngineChoice::Linked);
    let answer = Collie::scope(options, async |collie| find_cheapest(collie, replay).await).await?;

    match &answer.book {
        Some(book) => println!("title: {}\nprice: {:.2}", book.title, book.price),
        None => println!("no complete answer"),
    }
    println!("ended: {:?}", answer.ending);
    println!("steps: {}", answer.step_count);
    println!("spent: {}", answer.spent.as_deref().unwrap_or("unknown"));
    for line in &answer.steps {
        println!("  {line}");
    }
    Ok(())
}
