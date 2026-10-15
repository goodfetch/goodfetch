//! Snippet for A disk that fills under a long Run, and log volume (#553, ADR 0122), on scenario
//! 07's crawl.

use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlRun, CrawlRunReport, Error, ErrorCode, ExhaustedResource, Foxhound, Label,
    ResourcesExhausted, RunStopReason, Seed, StorageLevel,
};

/// A long crawl on a small disk: below `storage.critical_free` it stops resumably, never
/// half-written, and this same program run again under its ID carries on once space is freed.
async fn crawl_on_a_small_disk(books: &Crawl<'_>, home: &Label<()>) -> Result<Option<CrawlRunReport>, Error> {
    let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    let served = async {
        let run = foxhound
            .start_crawl(books, [Seed::new("https://books.toscrape.com/", home, ())])
            .run_id("books-2026-10-15")
            .await?;
        books.serve(&run).await
    }
    .await;
    let report = match served {
        Ok(report) => report,
        // Only where no report says it: a start refused on a full filesystem, or a stop it could
        // not record.
        Err(error) if error.code() == ErrorCode::RESOURCES_EXHAUSTED => {
            let details = error.details::<ResourcesExhausted>();
            eprintln!("storage full: {:?} ({:?})", details.map(|d| (d.path, d.free_bytes)), error.fix());
            foxhound.close().await?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if let Some(RunStopReason::Resources { resource: ExhaustedResource::Storage, path }) = &report.run.stop_reason {
        eprintln!("stopped before the disk filled: free space at {path:?}, then run again");
    }
    for warning in report.run.warnings.iter().filter(|w| w.code == "storage_low") {
        eprintln!("{}", warning.message);
    }
    foxhound.close().await?;
    Ok(Some(report))
}

/// While it runs: the worst storage level among the filesystems it writes to.
async fn watch_storage(run: &CrawlRun) -> Result<(), Error> {
    let mut progress = run.progress();
    while let Some(now) = progress.next().await {
        let storage = now?.run.storage;
        if storage.level != StorageLevel::Normal {
            eprintln!("storage {:?} at {:?}: {:?} bytes free", storage.level, storage.path, storage.free_bytes);
        }
    }
    Ok(())
}

fn main() {
    let _ = (crawl_on_a_small_disk, watch_storage);
}
