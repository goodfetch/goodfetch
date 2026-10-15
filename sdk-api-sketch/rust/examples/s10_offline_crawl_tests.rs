//! Golden scenario 10: the crawl of scenario 05 and the AI extraction of scenario 08, tested with no
//! network and no model: `cargo test --example s10_offline_crawl_tests --features test-util`. The
//! kit needs no harness of its own. Every Run the test engine starts replays a recording, an empty
//! one when it names none, so a request it lacks is never sent: it fails with
//! `response_not_recorded`, naming its address.

#[expect(
    dead_code,
    reason = "scenario 05's own `run` and `main` are not used here"
)]
#[path = "s05_two_page_crawl.rs"]
mod two_page_crawl;

#[expect(dead_code, reason = "scenario 08's own `main` is not used here")]
#[path = "s08_ai_in_crawl_handler.rs"]
mod ai_in_crawl_handler;

fn main() {}

#[cfg(test)]
mod tests {
    use goodfetch_foxhound::settings::plan;
    use goodfetch_foxhound::testing::{
        AttemptEnd, HandlerInput, SavedResponse, TestEngine, TestOptions,
    };
    use goodfetch_foxhound::{CrawlRunReport, Error, Recording, Seed, Settings};
    use serde_json::json;

    use super::ai_in_crawl_handler as ai;
    use super::two_page_crawl::{BookPayload, books_crawl};

    const BOOK: &str = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";
    /// One live run of scenario 05 recorded these, with `.recording(Recording::record(..))` on
    /// its start.
    const RECORDING: &str = "tests/books.warc.gz";

    /// The Records committed to the `books` Dataset.
    fn books_committed(report: &CrawlRunReport) -> Option<u64> {
        let (_, totals) = report.records.iter().find(|(name, _)| name == "books")?;
        Some(totals.committed)
    }

    #[tokio::test]
    async fn book_page_gives_its_book_with_the_category_passed_to_it() -> Result<(), Error> {
        let declared = books_crawl()?;
        let engine = TestEngine::start(TestOptions::default()).await?;
        // The handler alone, on a saved page: no Frontier, no browser on the `http` tier.
        let payload = BookPayload {
            category: "Poetry".into(),
        };
        let page = SavedResponse::path("tests/fixtures/a-light-in-the-attic.html");
        let input = HandlerInput::new(BOOK, payload).response(page);
        let attempt = engine
            .run_handler(&declared.crawl, &declared.book, input)
            .await?;
        let ended = &attempt.ended;
        assert!(matches!(ended, AttemptEnd::Returned), "{ended:?}");
        let records = serde_json::to_value(attempt.records(&declared.books)?).expect("JSON");
        let expected = json!([{
            "title": "A Light in the Attic",
            "price": "51.77",
            "in_stock": 22,
            "rating": 3,
            "url": BOOK,
            "category": "Poetry",
        }]);
        assert_eq!(records, expected);
        engine.close().await
    }

    #[tokio::test]
    async fn crawl_collects_every_book_from_the_recording() -> Result<(), Error> {
        let declared = books_crawl()?;
        let engine = TestEngine::start(TestOptions::default()).await?;
        let run = engine
            .connection()
            .start_crawl(&declared.crawl, declared.seeds())
            .recording(Recording::replay(RECORDING))
            .await?;
        let report = declared.crawl.serve(&run).await?; // no Politeness waits: nothing is sent
        let recording = report.run.recording.as_ref().expect("a replay reports it");
        let missed: Vec<&str> = recording.misses.iter().map(|m| m.url.as_str()).collect();
        assert!(missed.is_empty(), "not recorded: {missed:?}");
        assert_eq!(books_committed(&report), Some(1000));
        engine.close().await
    }

    #[tokio::test]
    async fn ai_extraction_replays_its_plan_with_no_model_call() -> Result<(), Error> {
        let declared = ai::crawl()?;
        // No Model provider: a model call would fail with `model_not_configured`.
        let engine = TestEngine::start(TestOptions::default()).await?;
        let run = engine
            .connection()
            .start_crawl(&declared.crawl, [Seed::new(BOOK, &declared.book, ())])
            .plan(ai::PLAN) // what scenario 08's live run learned, committed
            .recording(Recording::replay(RECORDING))
            // A step the Plan lacks fails with `plan_step_missing` instead of calling a model.
            .settings(Settings::new().set(plan::FROZEN, true))
            .await?;
        let report = declared.crawl.serve(&run).await?;
        let recording = report.run.recording.as_ref().expect("a replay reports it");
        let missed: Vec<&str> = recording.misses.iter().map(|m| m.url.as_str()).collect();
        assert!(missed.is_empty(), "not recorded: {missed:?}");
        assert_eq!(report.requests.failed, 0);
        assert_eq!(books_committed(&report), Some(1));
        assert_eq!(report.run.usage.model.calls, 0);
        engine.close().await
    }
}
