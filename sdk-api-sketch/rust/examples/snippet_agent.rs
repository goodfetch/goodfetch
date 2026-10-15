//! Snippet for the Agent in Collie's SDK API (#474, ADR 0100), on golden scenario 11.

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::{
    AgentResult, AgentTool, AgentToolCall, BoxError, ConnectOptions, Ended, Error, ErrorCode, Expected, Collie, Limits, Money, Page, Run,
    Schema, StopReason, Verdict, expect,
};
use serde::Deserialize;

/// The answer golden scenario 11 asks for.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Answer {
    /// The book's title.
    pub title: String,
    /// Its price, as the page writes it.
    #[schemars(regex(pattern = r"^£\d+\.\d{2}$"))]
    pub price: String,
}

/// The Success check: a book page in Travel with five stars. A model's "done" is only a claim.
async fn travel_book_check(page: Page) -> Result<(), BoxError> {
    expect(&page.locator(".breadcrumb")).to_contain_text("Travel").await?;
    expect(&page.locator(".product_main .star-rating")).to_have_class(Expected::regex(r"\bFive\b")).await?;
    Ok(())
}

/// Golden scenario 11, "Reach a goal from one instruction, with limits and a verdict". The Run caps
/// the price at 0.20 USD (a Limit, which stops the Run), the task caps itself at 25 steps (which
/// stops only the task), and the program, not the model, states what success looks like. With a
/// Plan, the steps are kept, and a later run repeats them with no model call.
async fn cheapest_five_star_travel_book() -> Result<Option<Answer>, Error> {
    let collie = Collie::connect(ConnectOptions::default()).await?;
    let run = collie
        .start()
        .plan("travel.plan.json")
        .limits(Limits::new().price(Money::new("0.20", "USD")?))
        .await?;
    let page = run.new_page().await?;
    page.goto("https://books.toscrape.com/").await?;

    let task = page
        .agent("find the cheapest book rated five stars in the Travel category and give its title and price")
        .schema(Schema::<Answer>::of())
        .max_steps(25)
        // By default the task stays on toscrape.com, where it starts; this narrows it to one host.
        .domains(["books.toscrape.com"])
        .success_check(travel_book_check)
        .start()
        .await?;

    // The steps as they happen, from step 1, none dropped.
    let mut steps = task.steps();
    while let Some(step) = steps.next().await {
        let step = step?;
        let did: Vec<&str> = step.actions.iter().map(|a| a.code.as_deref().unwrap_or(&a.operation)).collect();
        tracing_like(&format!("step {} ({:?}) at {}: {did:?}", step.index, step.origin, step.url));
    }
    let result = task.result().await?;
    run.close().await?;
    collie.close().await?;
    report(result)
}

/// How the work ended, its Verdict and the model's claim are three separate fields (ADR 0079).
fn report(result: AgentResult<Answer>) -> Result<Option<Answer>, Error> {
    if let Ended::Stopped(reason) = &result.ended {
        // `MaxSteps`, `NoProgress` and `Timeout` carry the setting that stopped it; `Limit` names the Limit.
        if let StopReason::MaxSteps(source) | StopReason::NoProgress(source) | StopReason::Timeout(source) = reason {
            tracing_like(&format!("stopped by {}", source.setting));
        }
        tracing_like(&format!("partial: {:?}", result.partial.as_ref().map(|p| p.json())));
        return Ok(None);
    }
    if result.verdict != Verdict::Verified {
        // Rejected: the Success check failed after the model said done; needs-review: nothing proved it.
        tracing_like(&format!("{:?}: {:?}", result.verdict, result.reported));
        return Ok(None);
    }
    result.require_verified().map(Some)
}

/// An Agent tool's input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Owned {
    /// The exact title.
    pub title: String,
}

/// A task awaited in one go, with a tool: the program's own function as an action, read-only.
async fn cheapest_unowned_book(run: &Run) -> Result<Answer, Error> {
    let owned = AgentTool::new("already_owned", "Whether the library already owns a book with this exact title", |input: Owned, _page: Page, _call: AgentToolCall| async move {
        let owned: Result<bool, BoxError> = Ok(input.title == "Sharp Objects");
        Ok(serde_json::json!({ "owned": owned? }))
    })
    .read_only();

    let page = run.new_page().await?;
    page.goto("https://books.toscrape.com/catalogue/category/books/travel_2/index.html").await?;
    let result = page
        .agent("find the cheapest five-star book here that the library does not own yet")
        .schema(Schema::<Answer>::of())
        .tool(owned)
        .success_check(travel_book_check)
        .await
        .inspect_err(|error| {
            if error.code() == ErrorCode::AGENT_TOOL_FAILED {
                tracing_like("a tool failed; its error is the source");
            }
            // Whatever ended the task, the steps taken and the Grounded partial answer travel on the error.
            if let Some(progress) = error.agent_task() {
                tracing_like(&format!("partial: {:?}", progress.partial));
            }
        })?;
    // Each page action carries its code in Rust, to turn the task into plain page API calls.
    let code: Vec<&str> = result.steps.iter().flat_map(|s| s.actions.iter().filter_map(|a| a.code.as_deref())).collect();
    tracing_like(&code.join("\n"));
    result.require_verified()
}

/// Stands in for the program's own `tracing` calls.
fn tracing_like(message: &str) {
    let _ = message;
}

fn main() {
    let _ = (cheapest_five_star_travel_book, cheapest_unowned_book);
}
