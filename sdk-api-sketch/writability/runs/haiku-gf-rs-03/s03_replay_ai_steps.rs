//! Reads an author's details from quotes.toscrape.com with plain-language AI steps (ADR 0091).
//!
//! The first run has the model resolve each step and keeps what it resolved in the Plan file next to
//! this program. A later run replays the Plan with no model call. When the site changes so that one
//! kept step no longer applies, only that step goes back to the model, and the output names it.

use std::fmt;
use std::process::ExitCode;

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::{
    Collie, ConnectOptions, Error, Expected, RunReport, Schema, StepReport, StepResult, StepSource,
    Verdict, expect,
};
use serde::Deserialize;

/// The Plan file, next to this program: read when the Run starts, written back when a new version
/// is promoted.
const PLAN_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/s03_replay_ai_steps.plan.json");

const QUOTES_HOME: &str = "https://quotes.toscrape.com/";
const AUTHOR_PREFIX: &str = "https://quotes.toscrape.com/author/";
const TAG_PAGE_PATTERN: &str = r"^https://quotes\.toscrape\.com/tag/love/";
const SECOND_PAGE_PATTERN: &str = r"^https://quotes\.toscrape\.com/tag/love/page/2/";
const AUTHOR_PATTERN: &str = r"^https://quotes\.toscrape\.com/author/";

/// One AI step: a name for people, and the instruction the Plan keys it by.
#[derive(Clone, Copy)]
struct StepSpec {
    name: &'static str,
    instruction: &'static str,
}

const OPEN_TAG: StepSpec = StepSpec {
    name: "open the tag \"love\" from the tag list",
    instruction: "open the tag named love in the tag list",
};

const SECOND_PAGE: StepSpec = StepSpec {
    name: "go to the second page of that tag",
    instruction: "go to the second page of the quotes for this tag",
};

const ABOUT_PAGE: StepSpec = StepSpec {
    name: "open the about page of the first quote's author",
    instruction: "on the first quote, open the about page of its author",
};

const AUTHOR_DETAILS: StepSpec = StepSpec {
    name: "read the author's details",
    instruction: "read the author's name, birth date and birthplace from this author page",
};

/// The author page's details, typed by the model's answer against this schema.
#[derive(Debug, Deserialize, JsonSchema)]
struct Author {
    /// The author's full name, as the page shows it.
    name: String,
    /// The birth date, as the page shows it.
    birth_date: String,
    /// The birthplace, as the page shows it.
    birthplace: String,
}

/// How one AI step was resolved, kept for the report.
struct StepRecord {
    spec: StepSpec,
    step: StepReport,
    model_calls: u64,
}

impl StepRecord {
    fn new<T>(spec: StepSpec, result: &StepResult<T>) -> Self {
        Self {
            spec,
            step: result.step.clone(),
            model_calls: result.usage.calls,
        }
    }
}

struct Outcome {
    author: Author,
    records: Vec<StepRecord>,
    report: RunReport,
}

/// Why the program stopped: a Collie call, with the step it belonged to, or a page that is not an
/// author page.
enum AppError {
    Collie {
        step: Option<&'static str>,
        error: Error,
    },
    NotAuthorPage {
        url: String,
    },
}

impl From<Error> for AppError {
    fn from(error: Error) -> Self {
        Self::Collie { step: None, error }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collie { step: Some(step), error } => write!(f, "step \"{step}\" failed: {error}"),
            Self::Collie { step: None, error } => write!(f, "run failed: {error}"),
            Self::NotAuthorPage { url } => write!(f, "expected an author page, reached {url}"),
        }
    }
}

fn at_step(spec: StepSpec) -> impl FnOnce(Error) -> AppError {
    move |error| AppError::Collie {
        step: Some(spec.name),
        error,
    }
}

async fn run_flow(collie: &Collie) -> Result<Outcome, AppError> {
    let run = collie.start().plan(PLAN_FILE).await?;
    let page = run.new_page().await?;
    let _ = page.goto(QUOTES_HOME).await?;

    let mut records = Vec::new();

    let tag = page.act(OPEN_TAG.instruction).await.map_err(at_step(OPEN_TAG))?;
    records.push(StepRecord::new(OPEN_TAG, &tag));
    expect(&page)
        .to_have_url(Expected::regex(TAG_PAGE_PATTERN))
        .await
        .map_err(at_step(OPEN_TAG))?;

    let second = page.act(SECOND_PAGE.instruction).await.map_err(at_step(SECOND_PAGE))?;
    records.push(StepRecord::new(SECOND_PAGE, &second));
    expect(&page)
        .to_have_url(Expected::regex(SECOND_PAGE_PATTERN))
        .await
        .map_err(at_step(SECOND_PAGE))?;

    let about = page.act(ABOUT_PAGE.instruction).await.map_err(at_step(ABOUT_PAGE))?;
    records.push(StepRecord::new(ABOUT_PAGE, &about));
    expect(&page)
        .to_have_url(Expected::regex(AUTHOR_PATTERN))
        .await
        .map_err(at_step(ABOUT_PAGE))?;

    // A plain string check on the address, with no model involved.
    let url = page.url();
    if !url.starts_with(AUTHOR_PREFIX) {
        return Err(AppError::NotAuthorPage { url });
    }

    let details = page
        .extract(AUTHOR_DETAILS.instruction, Schema::<Author>::of())
        .await
        .map_err(at_step(AUTHOR_DETAILS))?;
    records.push(StepRecord::new(AUTHOR_DETAILS, &details));
    expect(&page)
        .to_have_url(Expected::regex(AUTHOR_PATTERN))
        .await
        .map_err(at_step(AUTHOR_DETAILS))?;

    let report = run.finish().await?;
    Ok(Outcome {
        author: details.output,
        records,
        report,
    })
}

fn describe_source(source: &StepSource) -> String {
    match source {
        StepSource::Plan => "replayed from the kept Plan".to_owned(),
        StepSource::Model => "resolved by the model (no kept entry)".to_owned(),
        StepSource::Repair => "site changed: this step alone went back to the model".to_owned(),
        StepSource::Journal => "already done by an earlier run, not repeated".to_owned(),
        other => format!("{other:?}"),
    }
}

fn print_outcome(outcome: &Outcome) {
    println!("Name: {}", outcome.author.name);
    println!("Born: {}", outcome.author.birth_date);
    println!("Birthplace: {}", outcome.author.birthplace);
    println!();

    for record in &outcome.records {
        println!(
            "step \"{}\": {} ({} model calls)",
            record.spec.name,
            describe_source(&record.step.source),
            record.model_calls
        );
        if let Some(drift) = &record.step.drift {
            println!("  kept entry expected: {}", drift.expected);
            println!(
                "  page showed: {}",
                drift.observed.as_deref().unwrap_or("nothing matching")
            );
        }
    }

    let to_model: Vec<&str> = outcome
        .records
        .iter()
        .filter(|record| !matches!(record.step.source, StepSource::Plan))
        .map(|record| record.spec.name)
        .collect();
    let model_calls = outcome.report.usage.model.calls;
    if to_model.is_empty() && model_calls == 0 {
        println!(
            "All {} steps replayed from the Plan: no model call.",
            outcome.records.len()
        );
    } else {
        println!("Steps that went to the model: {}", to_model.join(", "));
        println!("Model calls this run: {model_calls}");
    }

    println!("Plan {PLAN_FILE}: {:?}", outcome.report.plan.status);
    match &outcome.report.verdict {
        Verdict::Verified => println!("Verdict: verified"),
        other => {
            println!("Verdict: {other:?}");
            for reason in &outcome.report.verdict_reasons {
                println!("  {}: {}", reason.code, reason.message);
            }
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let scoped = Collie::scope(
        ConnectOptions::default(),
        async |collie: &Collie| -> Result<Result<Outcome, AppError>, Error> {
            Ok(run_flow(collie).await)
        },
    )
    .await;

    let outcome = match scoped {
        Ok(result) => result,
        Err(error) => Err(AppError::from(error)),
    };

    match outcome {
        Ok(outcome) => {
            print_outcome(&outcome);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
