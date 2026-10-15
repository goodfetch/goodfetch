//! One instruction, one Agent goal: find the cheapest five-star Travel book on books.toscrape.com
//! and report its title and price.
//!
//! The program states the bounds (25 steps, a 0.20 USD price Limit, the books domain only) and the
//! Success check: the page reached is a Travel book page with a five-star rating. The Run's Plan
//! file keeps the steps taken, so the same task replays without a model call.

use std::error::Error as StdError;
use std::future::Future;
use std::time::Duration;

use goodfetch_collie::settings::ai as ai_settings;
use goodfetch_collie::{
    AgentResult, BoxError, Claim, Collie, ConnectOptions, EngineChoice, Ended, Expected, Limits,
    ModelDeclaration, ModelFormat, ModelProvider, ModelReference, Money, Page, Schema,
    SecretReference, Settings, StopReason, StructuredOutputStrategy, Verdict, expect,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The site the Agent is bound to; the Agent's navigations stay on it.
const BOOKS_HOST: &str = "books.toscrape.com";
/// Where the work starts.
const BOOKS_HOME: &str = "https://books.toscrape.com/";
/// The one instruction the Agent receives.
const TASK: &str = "On books.toscrape.com, find the cheapest book rated five stars in the Travel \
                    category and give its title and price.";
/// The step cap for the Agent's task.
const MAX_STEPS: u32 = 25;
/// The price Limit for the whole Run, in USD.
const PRICE_CAP_AMOUNT: &str = "0.20";
const PRICE_CAP_CURRENCY: &str = "USD";
/// The Run's own timeout.
const RUN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The Plan file: written when a verified Run promotes it, read to replay the task without a model.
const PLAN_FILE: &str = "books-travel-cheapest-five-star.plan.json";
/// The model provider the program declares, reached through the Vercel AI Gateway.
const PROVIDER: &str = "gateway";
/// The gateway's model ID, which keeps its own slash.
const MODEL_ID: &str = "anthropic/claude-haiku-4.5";
/// The gateway's OpenAI-compatible endpoint.
const GATEWAY_URL: &str = "https://ai-gateway.vercel.sh/v1";
/// The key, read by the engine from the environment: the program never holds its value.
const KEY_REFERENCE: &str = "env:AI_GATEWAY_API_KEY";
/// A book page's URL: a catalogue slug, an ID and the index page.
const BOOK_URL_PATTERN: &str = r"^https://books\.toscrape\.com/catalogue/[^/]+_[0-9]+/index\.html$";
/// The third breadcrumb item of a book page, which names its category.
const CATEGORY_CRUMB: &str = ".breadcrumb li:nth-child(3) a";
/// The rating marker of a five-star book.
const FIVE_STAR_MARKER: &str = ".star-rating.Five";
/// The category a book must be in.
const CATEGORY: &str = "Travel";

/// The typed answer the Agent fills in from the pages it observed.
#[derive(Debug, Deserialize, JsonSchema)]
struct Answer {
    /// The book's title, as the page shows it.
    title: String,
    /// The book's price, as the page shows it, with its currency sign.
    price: String,
}

/// How the work ended, read from the engine's own fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Ending {
    /// The program's Success check passed: a five-star Travel book page was reached.
    VerifiedByCondition,
    /// The model said the work was done, and no Success check proved it.
    ReportedDoneByModel,
    /// A Limit stopped the work, by name.
    StoppedByLimit { limit: String },
    /// Another stop reason ended the work, as the engine reports it.
    Stopped { reason: String },
    /// The work ended without a verified or claimed result.
    NotVerified,
}

/// What the program reports: the typed answer, how the work ended, and the steps taken as the
/// engine rendered them, so they can be repeated without a model.
#[derive(Debug, Serialize)]
struct Outcome {
    /// The title, when the work produced one.
    title: Option<String>,
    /// The price, when the work produced one.
    price: Option<String>,
    /// How the work ended.
    ending: Ending,
    /// The page actions of every step, in order.
    steps: Vec<String>,
}

/// Builds the gateway provider and the Settings that declare it and select its model.
fn settings() -> Result<Settings, Box<dyn StdError>> {
    let key: SecretReference = KEY_REFERENCE.parse()?;
    let provider = ModelProvider::new()
        .format(ModelFormat::OpenAi)
        .base_url(GATEWAY_URL)
        .api_key(key)
        .model(
            MODEL_ID,
            ModelDeclaration::new()
                .context_window(200_000)
                .max_output_tokens(8_192)
                .image_input(false)
                .tool_calls(true)
                .strategies(&[StructuredOutputStrategy::Tool, StructuredOutputStrategy::Prompt]),
        );
    Ok(Settings::new()
        .model_provider(PROVIDER, provider)
        .set(
            ai_settings::MODEL,
            ModelReference::from(format!("{PROVIDER}/{MODEL_ID}")),
        ))
}

/// The Success check: the page reached is a book page, in the Travel category, rated five stars.
fn verify_book_page(page: Page) -> impl Future<Output = Result<(), BoxError>> + Send + 'static {
    async move {
        expect(&page)
            .to_have_url(Expected::regex(BOOK_URL_PATTERN))
            .await?;
        expect(&page.locator(CATEGORY_CRUMB))
            .to_have_text(CATEGORY)
            .await?;
        expect(&page.locator(FIVE_STAR_MARKER))
            .to_be_visible()
            .await?;
        Ok::<(), BoxError>(())
    }
}

/// Reads the engine's fields for how the Agent's task ended. The Verdict decides first: a passed
/// Success check is proof, a model's claim is only a signal.
fn ending_of<T>(result: &AgentResult<T>) -> Ending {
    if result.verdict == Verdict::Verified {
        return Ending::VerifiedByCondition;
    }
    let claimed_done = result
        .reported
        .as_ref()
        .is_some_and(|report| report.claim == Claim::Done);
    match &result.ended {
        Ended::Stopped(StopReason::Limit(limit)) => Ending::StoppedByLimit {
            limit: limit.clone(),
        },
        Ended::Stopped(StopReason::MaxSteps(_)) => Ending::StoppedByLimit {
            limit: "agent.max_steps".to_owned(),
        },
        Ended::Stopped(StopReason::ModelAllowance(_)) => Ending::StoppedByLimit {
            limit: "ai.allowance".to_owned(),
        },
        Ended::Stopped(other) => Ending::Stopped {
            reason: format!("{other:?}"),
        },
        Ended::Finished if claimed_done => Ending::ReportedDoneByModel,
        _ => Ending::NotVerified,
    }
}

/// Runs the goal on one Run and one page, and returns the outcome. The Run is finished when the
/// Success check verified it, so its Plan is promoted; otherwise it is closed.
async fn find_cheapest(collie: &Collie) -> Result<Outcome, Box<dyn StdError>> {
    let price_cap = Money::new(PRICE_CAP_AMOUNT, PRICE_CAP_CURRENCY)?;
    let run = collie
        .start()
        .timeout(RUN_TIMEOUT)
        .limits(Limits::new().price(price_cap))
        .plan(PLAN_FILE)
        .await?;
    let page = run.new_page().await?;
    page.goto(BOOKS_HOME).await?;

    let result = page
        .agent(TASK)
        .schema(Schema::<Answer>::of())
        .success_check(verify_book_page)
        .max_steps(MAX_STEPS)
        .domains([BOOKS_HOST])
        .await?;

    let ending = ending_of(&result);
    let steps: Vec<String> = result
        .steps
        .iter()
        .flat_map(|step| step.actions.iter())
        .filter_map(|action| action.code.clone())
        .collect();
    let verified = result.verdict == Verdict::Verified;
    let (title, price) = match result.output {
        Some(answer) => (Some(answer.title), Some(answer.price)),
        None => (None, None),
    };

    if verified {
        let _report = run.finish().await?;
    } else {
        run.close().await?;
    }
    Ok(Outcome {
        title,
        price,
        ending,
        steps,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError>> {
    let collie = Collie::connect(
        ConnectOptions::default()
            .engine(EngineChoice::Linked)
            .settings(settings()?),
    )
    .await?;
    let outcome = find_cheapest(&collie).await;
    collie.close().await?;
    println!("{}", serde_json::to_string_pretty(&outcome?)?);
    Ok(())
}
