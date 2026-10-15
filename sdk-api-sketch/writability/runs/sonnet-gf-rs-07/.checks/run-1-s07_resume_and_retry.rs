//! Golden scenario 07: run scenario 05's crawl under a name, resume it after a kill, retry only its
//! failed requests, and say why nothing was done when nothing was left.
//!
//! `cargo run --example s07_resume_and_retry -- [RUN_NAME]`. The name defaults to `books-resumable`.
//! A start under a name that exists resumes that Run, or returns it ended; so the same command is
//! run again after a kill (it resumes), after the crawl ended with failures (it retries them) and
//! once more with nothing left (it says nothing was done, and why).

use std::collections::HashSet;

use goodfetch_foxhound::{
    ConnectOptions, CrawlRun, CrawlRunProgress, CrawlRunReport, CrawlRequestOutcomeKind, Error,
    ExportFormat, Foxhound, NothingDone, OutcomeKind, OutcomesFilter, StartedAs,
};

/// Scenario 05's crawl, included as a module.
#[expect(dead_code, reason = "scenario 05 is included whole; this program uses part of it")]
#[path = "s05_two_page_crawl.rs"]
mod two_page_crawl;

use two_page_crawl::{Book, BooksCrawl, books_crawl};

/// The Run's name when the command line gives none.
const DEFAULT_NAME: &str = "books-resumable";

/// Prints what is in flight and what waits, and why.
fn print_progress(progress: &CrawlRunProgress) {
    let counts = &progress.requests;
    println!(
        "progress: {} pending, {} fetching, {} handling, {} paused; ended {} succeeded, {} failed, {} skipped, {} blocked",
        counts.pending,
        counts.fetching,
        counts.handling,
        counts.paused,
        counts.succeeded,
        counts.failed,
        counts.skipped,
        counts.blocked
    );
    for flying in &progress.in_flight {
        let waiting = match &flying.waiting {
            Some((reason, _)) => format!(", waiting for a permit: {reason:?}"),
            None => String::new(),
        };
        println!(
            "  in flight {} ({:?} on {:?}, attempt {}){waiting}",
            flying.request.url, flying.phase, flying.tier, flying.attempt
        );
    }
    for group in &progress.waiting {
        let setting = match &group.setting {
            Some(source) => format!(" (setting {} = {:?}, from {:?})", source.setting, source.value, source.layer),
            None => String::new(),
        };
        println!("  waiting {:?}: {} requests{setting}", group.reason, group.requests);
        for domain in &group.domains {
            println!("    {}: {} requests, until {:?}", domain.domain, domain.requests, domain.until);
        }
        if group.more_domains > 0 {
            println!("    and {} more domains", group.more_domains);
        }
    }
    let concurrency = &progress.concurrency;
    println!(
        "  concurrency: {} handlers running (limit {} = {:?}), governor {}",
        concurrency.handlers_running,
        concurrency.handlers_limit.setting,
        concurrency.handlers_limit.value,
        concurrency.governor.level
    );
}

/// Serves the crawl until its Run ends, printing its progress meanwhile.
async fn serve_and_watch(declared: &BooksCrawl, run: &CrawlRun) -> Result<CrawlRunReport, Error> {
    let serve = declared.crawl.serve(run);
    tokio::pin!(serve);
    let mut progress = run.progress();
    let mut watching = true;
    loop {
        tokio::select! {
            report = &mut serve => return report,
            item = progress.next(), if watching => match item {
                Some(Ok(value)) => print_progress(&value),
                Some(Err(error)) => {
                    println!("progress stopped: {:?}: {}", error.code(), error.message());
                    watching = false;
                }
                None => watching = false,
            },
        }
    }
}

/// Prints the requests that ended failed, skipped or blocked.
async fn print_problems(run: &CrawlRun) -> Result<(), Error> {
    let kinds = [OutcomeKind::Failed, OutcomeKind::Skipped, OutcomeKind::Blocked];
    let mut outcomes = run.outcomes(OutcomesFilter::default().kinds(&kinds));
    while let Some(outcome) = outcomes.next().await {
        let outcome = outcome?;
        let url = &outcome.request.url;
        match &outcome.kind {
            CrawlRequestOutcomeKind::Failed(error) => {
                println!("failed {url}: {:?}: {}", error.code(), error.message());
            }
            CrawlRequestOutcomeKind::Skipped { reason, message } => {
                println!("skipped {url}: {reason:?} {}", message.as_deref().unwrap_or(""));
            }
            CrawlRequestOutcomeKind::Blocked(cause) => println!("blocked {url}: {cause:?}"),
            _ => {}
        }
    }
    Ok(())
}

/// Prints a report's request totals.
fn print_totals(report: &CrawlRunReport) {
    let totals = &report.requests;
    println!(
        "{} succeeded, {} failed, {} skipped, {} blocked, {} pending",
        totals.succeeded, totals.failed, totals.skipped, totals.blocked, totals.pending
    );
    if let Some(nothing_done) = &report.run.nothing_done {
        println!("{}", explain_nothing_done(nothing_done));
    }
}

/// Says in words why a Run did nothing.
fn explain_nothing_done(nothing_done: &NothingDone) -> String {
    match nothing_done {
        NothingDone::AlreadyEnded { ended_at } => {
            format!("nothing done: the Run had already ended (at {ended_at:?}) and was returned as it is")
        }
        NothingDone::NoSeedHandled { refused, skipped } => format!(
            "nothing done: no seed reached a handler (refused when added: {refused:?}; skipped when fetched: {skipped:?})"
        ),
        NothingDone::NothingPending => {
            "nothing done: the resume or retry found no request left to run".to_owned()
        }
        other => format!("nothing done: {other:?}"),
    }
}

/// Checks that the Dataset holds every book once, and exports it.
async fn check_and_export(declared: &BooksCrawl, run: &CrawlRun) -> Result<(), Error> {
    let mut records = run.records::<Book>(&declared.books);
    let mut seen = HashSet::new();
    let mut duplicates = 0u64;
    while let Some(record) = records.next().await {
        let record = record?;
        if !seen.insert(record.value.url) {
            duplicates += 1;
        }
    }
    println!("{} distinct books, {duplicates} duplicates", seen.len());
    let exported = run.export(&declared.books, ExportFormat::Json).await?;
    exported.save_as("books.json").await?;
    println!("exported {} books to books.json", exported.records());
    Ok(())
}

/// Starts, resumes or retries the named Run, as its state asks.
async fn run_named(foxhound: &Foxhound) -> Result<(), Error> {
    let name = std::env::args().nth(1).unwrap_or_else(|| DEFAULT_NAME.to_owned());
    let declared = books_crawl()?;
    let run = foxhound
        .start_crawl(&declared.crawl, declared.seeds())
        .id(&name)
        .await?;
    println!("run {name}: {:?}", run.started_as());

    // A New or Resumed Run still has requests to serve; an Ended one was returned as it is.
    let served = !matches!(run.started_as(), StartedAs::Ended);
    if served {
        let report = serve_and_watch(&declared, &run).await?;
        print_totals(&report);
    }

    // Retry only the requests that failed: skips are decisions, blocks are left alone.
    print_problems(&run).await?;
    let retried = run.retry_failed().await?;
    if retried > 0 {
        println!("retrying {retried} failed requests");
        let report = serve_and_watch(&declared, &run).await?;
        print_totals(&report);
        print_problems(&run).await?;
    } else if !served {
        // Nothing to serve and nothing to retry: say so, with the engine's reason.
        let report = run.finish().await?;
        match &report.run.nothing_done {
            Some(nothing_done) => println!("{}", explain_nothing_done(nothing_done)),
            None => println!(
                "nothing done: run {name} had already ended and no request failed, so there was nothing to retry"
            ),
        }
    }

    check_and_export(&declared, &run).await
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    Foxhound::scope(ConnectOptions::default(), run_named).await
}
