//! Opens the tag "love" on quotes.toscrape.com, goes to its second page and opens the first quote's
//! author page, using steps written in plain words.
//!
//! Each step is one AI verb of the Run, kept in a Plan file next to this program. The first run has
//! the model compile every step; a second run replays them all from the file with no model call. If
//! the site changes so that one kept step no longer applies, only that step is compiled again by the
//! model, and the program prints which step it was.
//!
//! The model is named by the `QUOTES_MODEL` environment variable (default below); it is only called
//! for a step the Plan does not hold or one that drifted.

use std::path::PathBuf;

use goodfetch_collie::schemars::JsonSchema;
use goodfetch_collie::settings::{Settings, ai};
use goodfetch_collie::{
    Collie, ConnectOptions, Error, Expected, ModelReference, PlanDrift, PlanStepMissing, Schema,
    StepResult, StepSource, TargetNotFound, expect,
};
use serde::{Deserialize, Serialize};

const START_URL: &str = "https://quotes.toscrape.com/";
const AUTHOR_URL_PATTERN: &str = "^https://quotes\\.toscrape\\.com/author/";
const DEFAULT_MODEL: &str = "vercel/anthropic/claude-sonnet-4.5";

const STEP_OPEN_TAG: &str = "Click the tag \"love\" in the list of tags";
const STEP_SECOND_PAGE: &str = "Go to the second page of the results by clicking the Next link";
const STEP_OPEN_ABOUT: &str = "Click the \"(about)\" link next to the author of the first quote";
const STEP_READ_AUTHOR: &str =
    "Read the author's name, birth date and birthplace from this author page";

/// What the author page tells about its author.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "goodfetch_collie::schemars")]
struct Author {
    /// The author's full name.
    name: String,
    /// The birth date, as the page writes it.
    birth_date: String,
    /// The birthplace, as the page writes it.
    birthplace: String,
}

/// Prints how a step was resolved, says so when it went back to the model, and returns its output.
fn note<T>(result: StepResult<T>) -> T {
    let instruction = result.step.instruction.clone();
    match &result.step.source {
        StepSource::Plan => println!("step kept in the plan, no model call: {instruction}"),
        StepSource::Model => println!("step compiled by the model (not in the plan): {instruction}"),
        StepSource::Repair => {
            let seen = result
                .step
                .drift
                .as_ref()
                .map(|drift| format!(" (expected {}, found {:?})", drift.expected, drift.observed))
                .unwrap_or_default();
            println!("step no longer applied and went back to the model: {instruction}{seen}");
        }
        other => println!("step resolved as {other:?}: {instruction}"),
    }
    result.output
}

/// Names the step an error concerns, when the error is about one.
fn name_failed_step(error: &Error) {
    if let Some(drift) = error.details::<PlanDrift>() {
        eprintln!("step that no longer applies and could not be repaired: {}", drift.instruction);
    } else if let Some(missing) = error.details::<PlanStepMissing>() {
        eprintln!("step missing from the plan: {}", missing.instruction);
    } else if let Some(not_found) = error.details::<TargetNotFound>() {
        eprintln!(
            "step that found nothing on the page: {}",
            not_found.instruction.unwrap_or_default()
        );
    }
}

async fn steps(collie: &Collie, plan_file: PathBuf) -> Result<Author, Error> {
    let model = std::env::var("QUOTES_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    let run = collie
        .start()
        .plan(plan_file)
        .settings(Settings::new().set(ai::MODEL, ModelReference::from(model)))
        .await?;

    let page = run.new_page().await?;
    page.goto(START_URL).await?;

    note(page.act(STEP_OPEN_TAG).await?);
    note(page.act(STEP_SECOND_PAGE).await?);
    note(page.act(STEP_OPEN_ABOUT).await?);

    // A Success check the engine evaluates itself, without a model.
    expect(&page).to_have_url(Expected::regex(AUTHOR_URL_PATTERN)).await?;

    let author = note(page.extract(STEP_READ_AUTHOR, Schema::<Author>::of()).await?);

    let report = run.finish().await?;
    println!(
        "plan {:?}; steps compiled or repaired in this run: {:?}",
        report.plan.status, report.plan.changed_steps
    );
    Ok(author)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The Plan file lives next to this program's source.
    let plan_file = PathBuf::from(file!()).with_extension("plan.json");

    let collie = Collie::connect(ConnectOptions::default()).await?;
    let outcome = steps(&collie, plan_file).await;
    collie.close().await?;

    match outcome {
        Ok(author) => {
            println!("{}", serde_json::to_string_pretty(&author)?);
            Ok(())
        }
        Err(error) => {
            name_failed_step(&error);
            Err(error.into())
        }
    }
}
