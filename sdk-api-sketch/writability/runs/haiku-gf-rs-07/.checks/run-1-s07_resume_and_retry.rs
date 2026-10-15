//! Scenario 07: the books crawl of scenario 05, run under a name so that it resumes after a kill,
//! and whose failed requests can be retried in the same Run.
//!
//! Started normally, the program crawls under [`RUN_NAME`]. Killed part-way, a start under the same
//! name resumes the Run from its committed state, and the `books` Dataset, keyed by URL, keeps each
//! book once. Started with `retry` as its argument, the program first re-admits, in the same Run,
//! only the requests that ended `failed`. Whatever the start, the program says what the Run did,
//! and when it did nothing, why.

// Scenario 05's crawl and its handles. Its own `run` and `main` are not used by this program.
#[path = "s05_two_page_crawl.rs"]
#[expect(dead_code, reason = "scenario 05's own run and main are not used by this program")]
mod two_page_crawl;

use std::collections::{BTreeMap, BTreeSet};
use std::pin::pin;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlRequestOutcomeKind, CrawlRun, CrawlRunProgress, CrawlRunReport,
    Dataset, Error, ExportFormat, Foxhound, NothingDone, OutcomesFilter, StartedAs,
};
use two_page_crawl::{Book, books_crawl};

/// The name of the crawl's Run: a start under it resumes the Run, and a retry continues it.
const RUN_NAME: &str = "books-crawl-07";

/// How often a progress snapshot is printed while the crawl runs.
const PROGRESS_EVERY: Duration = Duration::from_secs(5);

/// Runs the crawl under [`RUN_NAME`]: with `retry` as the argument, re-admits its failed requests
/// first, then serves the Run and reports what it did.
async fn run(foxhound: &Foxhound) -> Result<(), Error> {
    let retry = std::env::args().nth(1).as_deref() == Some("retry");
    let declared = books_crawl()?;
    let run = foxhound
        .start_crawl(&declared.crawl, declared.seeds())
        .id(RUN_NAME)
        .await?;
    match run.started_as() {
        StartedAs::New => println!("run {RUN_NAME}: started"),
        StartedAs::Resumed => println!("run {RUN_NAME}: resumed where it stopped"),
        StartedAs::Ended => println!("run {RUN_NAME}: had already ended, so it is returned as it is"),
        other => println!("run {RUN_NAME}: started as {other:?}"),
    }
    if retry {
        let admitted = run.retry_failed().await?;
        if admitted == 0 {
            println!("retry: no request ended failed, so none was re-admitted");
        } else {
            println!("retry: re-admitted {admitted} failed requests");
        }
    }
    let report = serve_with_progress(&declared.crawl, &run).await?;
    print_totals(&report);
    print_failures(&run).await?;
    summarize_dataset(&run, &declared.books).await?;
    if let Some(nothing) = &report.run.nothing_done {
        println!("nothing done: {}", explain_nothing_done(nothing));
    }
    Ok(())
}

/// Serves the crawl's requests until the Run ends, printing progress every few seconds: what is in
/// flight, what waits and why, and which Session waits for a person.
async fn serve_with_progress(crawl: &Crawl<'_>, run: &CrawlRun) -> Result<CrawlRunReport, Error> {
    let mut progress = run.progress();
    let mut progress_open = true;
    let mut last_shown: Option<Instant> = None;
    let mut serving = pin!(crawl.serve(run));
    let report = loop {
        tokio::select! {
            report = &mut serving => break report?,
            next = progress.next(), if progress_open => match next {
                Some(snapshot) => {
                    let snapshot = snapshot?;
                    if last_shown.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
                        show_progress(&snapshot);
                        last_shown = Some(Instant::now());
                    }
                }
                None => progress_open = false,
            },
        }
    };
    Ok(report)
}

/// Prints one progress snapshot: the requests by state, what is in flight, what waits and why.
fn show_progress(progress: &CrawlRunProgress) {
    let counts = &progress.requests;
    println!(
        "progress: pending {}, fetching {}, handling {}, paused {}; succeeded {}, failed {}, skipped {}, blocked {}",
        counts.pending,
        counts.fetching,
        counts.handling,
        counts.paused,
        counts.succeeded,
        counts.failed,
        counts.skipped,
        counts.blocked
    );
    for flight in &progress.in_flight {
        let url = &flight.request.url;
        let phase = &flight.phase;
        let attempt = flight.attempt;
        let tier = &flight.tier;
        let held = match &flight.waiting {
            Some((reason, _)) => format!(", waiting on {reason:?}"),
            None => String::new(),
        };
        println!("  in flight: {url} ({phase:?}, attempt {attempt}, tier {tier:?}{held})");
    }
    for group in &progress.waiting {
        println!("  waiting: {} requests held by {:?}", group.requests, group.reason);
        for domain in &group.domains {
            println!("    {} ({} requests)", domain.domain, domain.requests);
        }
        if group.more_domains > 0 {
            println!("    and {} more domains", group.more_domains);
        }
    }
    for pause in &progress.run.pauses {
        println!(
            "  paused: session {} waits for a person ({:?})",
            pause.session, pause.reason
        );
    }
}

/// Prints the requests the Run ended, by state.
fn print_totals(report: &CrawlRunReport) {
    let totals = &report.requests;
    println!(
        "{} succeeded, {} failed, {} skipped, {} blocked, {} pending",
        totals.succeeded, totals.failed, totals.skipped, totals.blocked, totals.pending
    );
}

/// Lists the requests whose latest outcome is `failed`. A retried request's new outcome supersedes
/// its old one, keyed by its ID, so the latest per request decides.
async fn print_failures(run: &CrawlRun) -> Result<(), Error> {
    let mut latest = BTreeMap::new();
    let mut outcomes = run.outcomes(OutcomesFilter::default());
    while let Some(outcome) = outcomes.next().await {
        let outcome = outcome?;
        let failure = match &outcome.kind {
            CrawlRequestOutcomeKind::Failed(error) => {
                Some(format!("{:?}: {}", error.code(), error.message()))
            }
            _ => None,
        };
        latest.insert(outcome.request.id, (outcome.request.url, failure));
    }
    for (url, failure) in latest.values() {
        if let Some(failure) = failure {
            println!("failed {url}: {failure}");
        }
    }
    Ok(())
}

/// Reads the Dataset back, counting its Records and distinct URLs, then exports it to `books.json`.
async fn summarize_dataset(run: &CrawlRun, books: &Dataset<Book>) -> Result<(), Error> {
    let mut records = run.records(books);
    let mut count = 0_u64;
    let mut urls = BTreeSet::new();
    while let Some(record) = records.next().await {
        let record = record?;
        count += 1;
        urls.insert(record.value.url);
    }
    println!("dataset books: {count} records, {} distinct URLs", urls.len());
    let exported = run.export(books, ExportFormat::Json).await?;
    exported.save_as("books.json").await?;
    println!("exported {} books to books.json", exported.records());
    Ok(())
}

/// Why a start did nothing, in words a person reads.
fn explain_nothing_done(nothing: &NothingDone) -> String {
    match nothing {
        NothingDone::AlreadyEnded { ended_at } => format!(
            "the run had already ended at Unix time {}s, so this start had no work left to do",
            unix_seconds(ended_at)
        ),
        NothingDone::NoSeedHandled { refused, skipped } => format!(
            "no seed reached a handler: refused when added {refused:?}, skipped when fetched {skipped:?}"
        ),
        NothingDone::NothingPending => {
            "no request was pending, so the resume or retry found nothing left to do".to_owned()
        }
        other => format!("the engine reported {other:?}"),
    }
}

/// A time as whole seconds since the Unix epoch, 0 before it.
fn unix_seconds(time: &SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    Foxhound::scope(ConnectOptions::default(), run).await
}
