//! Snippet for The Run in code (#478, ADR 0102), on golden scenario 07 with scenario 05's crawl.

use std::time::Duration;

use goodfetch_sdk_sketch::settings::{checkpoint, crawl};
use goodfetch_sdk_sketch::output::ExportFormat;
use goodfetch_sdk_sketch::{
    ConnectOptions, Crawl, CrawlRun, CrawlRunReport, Dataset, Enqueued, Error, Goodfetch, Label, NothingDone, OutcomeKind,
    OutcomesFilter, Run, Seed, Settings, StartedAs,
};

/// Scenario 07: one program, run again under the same ID after a kill, then with `retry_failed`,
/// then once more with nothing left to do. `books` is scenario 05's declaration (snippet_crawl.rs).
async fn scenario_07(books: &Crawl<'_>, home: &Label<()>, retry_failed: bool) -> Result<CrawlRunReport, Error> {
    let gf = Goodfetch::connect(ConnectOptions::default()).await?;
    let run = gf
        .foxhound()
        .start_crawl(books, [Seed::new("https://books.toscrape.com/", home, ())])
        .id("books-2026-10-15") // a start under this ID resumes the Run where it stopped
        .await?;
    if run.started_as() == &StartedAs::Resumed {
        // carrying on where the killed process stopped
    }
    for answer in run.seeds() {
        if let Enqueued::NotAdded { url, reason } = answer {
            let _ = (url, reason); // a seed the domain policy refused, say
        }
    }
    if retry_failed {
        let readmitted = run.retry_failed().await?; // only `failed`, in this same Run and its Dataset
        let _ = readmitted;
    }
    let report = books.serve(&run).await?; // answers until the Run ends, then its report
    if let Some(NothingDone::AlreadyEnded { ended_at }) = &report.run.nothing_done {
        let _ = ended_at; // finished earlier: nothing was done
    }
    gf.close().await?;
    Ok(report)
}

/// While it runs: what is in flight, what waits, and why.
async fn follow(run: &CrawlRun) -> Result<(), Error> {
    let mut progress = run.progress();
    while let Some(now) = progress.next().await {
        let now = now?;
        let _ = (now.requests.pending, now.in_flight.len(), now.concurrency.handlers_running, &now.concurrency.handlers_limit.setting);
        for wait in &now.waiting {
            let _ = (&wait.reason, wait.requests, wait.setting.as_ref().map(|s| &s.setting));
        }
    }
    Ok(())
}

/// The failed URLs, from the Run's committed state, also once it has ended.
async fn failed_urls(run: &CrawlRun) -> Result<Vec<String>, Error> {
    let mut outcomes = run.outcomes(OutcomesFilter::default().kinds(&[OutcomeKind::Failed]));
    let mut urls = Vec::new();
    while let Some(outcome) = outcomes.next().await {
        urls.push(outcome?.request.url);
    }
    Ok(urls)
}

/// Yesterday's crawl, read by its ID from another program: nothing resumed, nothing driven.
async fn export_yesterday(gf: &Goodfetch, books: &Dataset<serde_json::Value>) -> Result<(), Error> {
    let view = gf.foxhound().open_crawl_run("books-2026-10-14").await?;
    let report = view.report().await?; // at once: it has ended
    let _ = (&report.records, &report.sinks);
    let csv = view.export(books, ExportFormat::Csv).await?;
    csv.save_as("books.csv").await?;
    view.close().await
}

/// A Collie Run under an ID: run again after a crash, it finds its Sessions and its Effect journal,
/// so an order placed before the crash is checked rather than placed again; once finished, it runs
/// nothing.
async fn place_order(gf: &Goodfetch, order_id: &str) -> Result<(), Error> {
    let run: Run = gf.collie().start().id(format!("order-{order_id}")).await?;
    if run.started_as() == &StartedAs::Ended {
        return run.close().await;
    }
    run.set_progress_message(&format!("order {order_id}")).await?;
    let page = run.new_page().await?;
    page.goto("https://shop.example/cart").await?;
    page.act("place the order").await?;
    let report = run.finish().await?; // `finished` and `complete`; the Verdict comes from its checks
    let _ = (report.verdict, report.usage.price, report.weakened.len());
    Ok(())
}

/// A keep-alive crawl fed while it runs, finished by the program from another future while `serve`
/// answers (ADR 0037's RT-12).
async fn fed(gf: &Goodfetch, books: &Crawl<'_>, home: &Label<()>, urls: Vec<String>) -> Result<(), Error> {
    let run = gf
        .foxhound()
        .start_crawl(books, [Seed::new("https://books.toscrape.com/", home, ())])
        .settings(Settings::new().set(crawl::KEEP_ALIVE, true).set(checkpoint::INTERVAL, Some(Duration::from_secs(600))))
        .await?;
    let feed = async {
        for url in &urls {
            run.add_seed(url, home, ()).await?;
        }
        let checkpoint = run.as_run().checkpoint().path("/mnt/backup/books.checkpoint").await?;
        let _ = (checkpoint.digest, checkpoint.size);
        run.finish().await // ends once what is pending is done
    };
    let (served, finished) = futures_util::join!(books.serve(&run), feed);
    let _ = (served?, finished?);
    Ok(())
}

fn main() {
    let _ = (scenario_07, follow, failed_urls, export_yesterday, place_order, fed);
}
