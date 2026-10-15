//! Golden scenario 11, "Reach a goal from one instruction, with limits and a verdict" (ADRs 0013,
//! 0079, 0091, 0100, 0101).

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::{
    AgentResult, BoxError, Claim, Collie, ConnectOptions, Ended, Error, Expected, Limits, Money,
    Page, PartialOutput, Schema, StopReason, Verdict, expect,
};
use serde::Deserialize;

/// The answer the task asks for.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Answer {
    /// The book's title.
    pub title: String,
    /// Its price, as the page writes it.
    #[schemars(regex(pattern = r"^£\d+\.\d{2}$"))]
    pub price: String,
}

/// How the work ended, with what it found.
#[derive(Debug)]
pub enum Outcome {
    /// The program's Success check held on the page the task reached.
    Verified(Answer),
    /// The work finished with a complete answer, and nothing proved it.
    Reported {
        /// What the model claimed, if anything.
        claim: Option<Claim>,
        /// Its answer.
        answer: Answer,
    },
    /// A limit stopped it, or the model gave up.
    Stopped {
        /// Why, naming the setting or the Limit.
        reason: StopReason,
        /// The Grounded part of the answer found so far.
        partial: PartialOutput,
    },
}

/// The Success check: the model's "done" is only a claim until this holds on the page it reached.
async fn travel_book_check(page: Page) -> Result<(), BoxError> {
    expect(&page.locator(".breadcrumb"))
        .to_contain_text("Travel")
        .await?;
    expect(&page.locator(".product_main .star-rating"))
        .to_have_class(Expected::regex(r"\bFive\b"))
        .await?;
    Ok(())
}

fn outcome(result: AgentResult<Answer>) -> Result<Outcome, Error> {
    if result.verdict == Verdict::Verified {
        return result.require_verified().map(Outcome::Verified);
    }
    // The answer is inside how the task ended: complete only when it finished.
    Ok(match result.ended {
        Ended::Finished { output } => Outcome::Reported {
            claim: result.reported.map(|reported| reported.claim),
            answer: output,
        },
        Ended::Stopped { reason, partial } => Outcome::Stopped { reason, partial },
    })
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // No telemetry exists to switch off: only the site and the model's provider are reached (ADR
    // 0043). Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic.
    let result = Collie::scope(ConnectOptions::default(), async |collie| {
        let run = collie
            .start()
            // The task is one Plan entry: once a Run is verified, a later one replays it with no
            // model call.
            .plan("travel.plan.json")
            // A Limit: reaching it stops the Run too.
            .limits(Limits::new().price(Money::new("0.20", "USD")?))
            .await?;
        let page = run.new_page().await?;
        page.goto("https://books.toscrape.com/").await?;
        let result = page
            .agent("find the cheapest book rated five stars in the Travel category and give its title and price")
            .schema(Schema::<Answer>::of())
            .model("anthropic/claude-sonnet-4-5")
            // Stops only the task.
            .max_steps(25)
            // Enforced by the engine on every navigation the task makes, whatever the pages say
            // (ADRs 0014, 0100).
            .domains(["books.toscrape.com"])
            .success_check(travel_book_check)
            .await?;
        let _ = run.finish().await?;
        Ok(result)
    })
    .await?;
    println!("{:?}", outcome(result)?);
    Ok(())
}
