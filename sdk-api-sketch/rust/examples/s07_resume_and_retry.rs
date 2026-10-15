//! Golden scenario 07: the crawl of scenario 05 under an ID of the program's choosing. Run it,
//! kill it, run it again: the start resumes the Run; once crawled, it retries only the failed
//! requests; a third run finds the Run ended and says so.

#[expect(
    dead_code,
    reason = "scenario 05's own `run` and `main` are not used here"
)]
#[path = "s05_two_page_crawl.rs"]
mod two_page_crawl;

use std::time::{Duration, Instant};

use goodfetch_foxhound::{
    ConnectOptions, CrawlRun, CrawlRunReport, Error, ExportFormat, Foxhound, Lifecycle, StartedAs,
};

const RUN_ID: &str = "books-toscrape-2026-10-15";

/// What is in flight and what waits, and why, every five seconds at most.
async fn print_progress(run: &CrawlRun) -> Result<(), Error> {
    let mut printed_at: Option<Instant> = None;
    let mut progress = run.progress();
    while let Some(now) = progress.next().await {
        let now = now?;
        if printed_at.is_some_and(|at| at.elapsed() < Duration::from_secs(5)) {
            continue;
        }
        printed_at = Some(Instant::now());
        let counts = &now.requests;
        println!(
            "{} pending, {} succeeded, {} failed, {} skipped",
            counts.pending, counts.succeeded, counts.failed, counts.skipped
        );
        for flight in &now.in_flight {
            let (url, phase, tier) = (&flight.request.url, &flight.phase, &flight.tier);
            println!(
                "  in flight: {url} ({phase:?} over {tier:?}, attempt {})",
                flight.attempt
            );
        }
        for wait in &now.waiting {
            let setting = wait
                .setting
                .as_ref()
                .map(|s| (&s.setting, s.value.as_json()));
            println!(
                "  waiting: {} for {:?} {setting:?}",
                wait.requests, wait.reason
            );
        }
    }
    Ok(())
}

/// Serves the crawl until its Run ends, printing its progress, or stops it on Ctrl+C: dropping
/// `serve` is the program's own stop, so the requests in flight return to pending, and `stop` is
/// bounded by `timeout::CLOSE`.
async fn crawl_to_end(
    declared: &two_page_crawl::BooksCrawl,
    run: &CrawlRun,
) -> Result<CrawlRunReport, Error> {
    tokio::select! {
        served = async { tokio::try_join!(declared.crawl.serve(run), print_progress(run)) } => {
            Ok(served?.0)
        }
        _ = tokio::signal::ctrl_c() => run.stop().await,
    }
}

async fn resume_and_retry(foxhound: &Foxhound) -> Result<(), Error> {
    let declared = two_page_crawl::books_crawl()?;
    // The same ID begins the Run, resumes it after a kill, or returns it ended. A changed program
    // (seeds, a Label gone, a Payload schema changed) or a damaged run file fails with its code.
    let run = foxhound
        .start_crawl(&declared.crawl, declared.seeds())
        .id(RUN_ID)
        .await?;
    println!("run {}: {:?}", run.id(), run.started_as());

    let mut report = crawl_to_end(&declared, &run).await?;
    if report.run.lifecycle == Lifecycle::Finished && run.started_as() != &StartedAs::Ended {
        let readmitted = run.retry_failed().await?; // only the failed requests, in this Run
        println!("retrying {readmitted} failed requests");
        if readmitted > 0 {
            report = crawl_to_end(&declared, &run).await?;
        }
    }
    if let Some(nothing_done) = &report.run.nothing_done {
        println!("nothing done: {nothing_done:?}");
    }
    let totals = &report.requests;
    println!(
        "{:?}: {} succeeded, {} failed, {} skipped",
        report.run.lifecycle, totals.succeeded, totals.failed, totals.skipped
    );
    if report.run.lifecycle == Lifecycle::Finished {
        // Each book once: Records commit with their attempt.
        let exported = run.export(&declared.books, ExportFormat::Json).await?;
        exported.save_as("books.json").await?;
        println!("exported {} books to books.json", exported.records());
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    Foxhound::scope(ConnectOptions::default(), resume_and_retry).await
}
