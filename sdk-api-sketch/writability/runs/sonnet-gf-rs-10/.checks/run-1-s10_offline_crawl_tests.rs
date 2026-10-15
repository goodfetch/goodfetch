//! Golden scenario 10: offline tests of the crawl of scenario 05 and the AI extraction of scenario
//! 08, run with `cargo test --example s10_offline_crawl_tests`: no network and no model.
//!
//! Everything runs on [`TestEngine`], which is offline: a Run that names no Response recording
//! replays an empty one, so a request to a site is refused with `response_not_recorded`, naming it,
//! and no model provider exists unless a test registers a Scripted one. The tests turn every such
//! refusal into a failure that says which address was tried.
//!
//! Fixtures, saved once from a live run (the recording with `Recording::record`, the Plan by
//! running scenario 08 with `.plan(..)`), live under `tests/fixtures/` and are read relative to
//! the working directory:
//!
//! - `book_page.html`: one book page, saved from a browser.
//! - `books.warc`: every response of one live run of scenario 05's crawl.
//! - `books_ai.warc` and `books.plan.json`: the same for scenario 08's crawl, and what it learned.

#[allow(dead_code, reason = "scenario 05's program is included for its declaration, not its main")]
#[path = "s05_two_page_crawl.rs"]
mod two_page_crawl;

#[allow(dead_code, reason = "scenario 08's program is included for its declaration, not its main")]
#[path = "s08_ai_in_crawl_handler.rs"]
mod ai_in_crawl_handler;

/// The programs are scenarios 05 and 08, run by their own mains; this example is its tests.
#[cfg_attr(test, allow(dead_code, reason = "the test harness has its own entry point"))]
fn main() {}

#[cfg(test)]
mod tests {
    use goodfetch_foxhound::settings::{ai, extract, plan};
    use goodfetch_foxhound::testing::{
        AttemptEnd, HandlerInput, SavedResponse, ScriptedModel, ScriptedReply, TestEngine,
        TestOptions,
    };
    use goodfetch_foxhound::{
        CrawlRun, CrawlRunReport, Dataset, Error, ExtractMode, ModelResponse, PlanStatus,
        Recording, RecordingMode, ReportedTokens, ResponseNotRecorded, Settings,
    };
    use serde::de::DeserializeOwned;

    use crate::ai_in_crawl_handler::{self, BookFields};
    use crate::two_page_crawl::{self, BookPayload};

    /// One book page, saved from a browser.
    const BOOK_PAGE: &str = "tests/fixtures/book_page.html";
    /// The book page's address, which is also the request's.
    const BOOK_URL: &str =
        "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";
    /// Every response of one live run of scenario 05's crawl.
    const BOOKS_RECORDING: &str = "tests/fixtures/books.warc";
    /// The books on the site when that run was made.
    const RECORDED_BOOKS: usize = 1000;
    /// Every response of one live run of scenario 08's crawl.
    const AI_RECORDING: &str = "tests/fixtures/books_ai.warc";
    /// What that run learned.
    const LEARNED_PLAN: &str = "tests/fixtures/books.plan.json";
    /// Scenario 08 keeps the first three categories: Travel (11 books), Mystery (32) and Historical
    /// Fiction (26).
    const RECORDED_AI_BOOKS: usize = 11 + 32 + 26;

    /// The address a refused request named, to say it in a failure.
    fn address_tried(error: &Error) -> String {
        match error.details::<ResponseNotRecorded>() {
            Some(refused) => format!("{} {}", refused.method, refused.url),
            None => format!("{}: {}", error.code().as_str(), error.message()),
        }
    }

    /// Fails the test, naming each address the Run tried to reach that its recording lacks.
    fn assert_no_network(report: &CrawlRunReport) {
        let recording = report
            .run
            .recording
            .as_ref()
            .expect("the Run must replay a recording, not reach the network");
        assert_eq!(recording.mode, RecordingMode::Replay, "the Run must replay");
        let tried: Vec<String> = recording
            .misses
            .iter()
            .map(|miss| format!("{} {}", miss.method, miss.url))
            .collect();
        assert!(
            tried.is_empty(),
            "the test tried to reach the network at: {}",
            tried.join(", ")
        );
        assert_eq!(
            report.requests.failed, 0,
            "requests failed: {:?}",
            report.requests.by_error_code
        );
    }

    /// Every Record the Dataset committed.
    async fn committed<T: DeserializeOwned>(run: &CrawlRun, dataset: &Dataset<T>) -> Vec<T> {
        let mut records = run.records(dataset);
        let mut values = Vec::new();
        while let Some(record) = records.next().await {
            values.push(record.expect("a committed Record reads back").value);
        }
        values
    }

    /// An engine with no model provider and no network.
    async fn offline_engine() -> TestEngine {
        TestEngine::start(TestOptions::default())
            .await
            .expect("the test engine starts")
    }

    #[tokio::test]
    async fn book_handler_produces_the_item_with_the_category_it_was_given() {
        let declared = two_page_crawl::books_crawl().expect("the crawl declares");
        let engine = offline_engine().await;
        let input = HandlerInput::new(
            BOOK_URL,
            BookPayload {
                category: "Poetry".to_owned(),
            },
        )
        .response(SavedResponse::path(BOOK_PAGE));

        let attempt = engine
            .run_handler(&declared.crawl, &declared.book, input)
            .await
            .unwrap_or_else(|error| panic!("the handler did not run: {}", address_tried(&error)));

        assert!(
            matches!(attempt.ended, AttemptEnd::Returned),
            "the handler ended {:?}",
            attempt.ended
        );
        let books = attempt.records(&declared.books).expect("the Records read");
        assert_eq!(books.len(), 1);
        let book = &books[0];
        assert_eq!(book.title, "A Light in the Attic");
        assert_eq!(book.price, "51.77");
        assert_eq!(book.in_stock, 22);
        assert_eq!(book.rating, 3);
        assert_eq!(book.url, BOOK_URL);
        assert_eq!(book.category, "Poetry");
        engine.close().await.expect("the engine closes");
    }

    #[tokio::test]
    async fn whole_crawl_from_the_recorded_responses_holds_every_book() {
        let declared = two_page_crawl::books_crawl().expect("the crawl declares");
        let engine = offline_engine().await;
        let foxhound = engine.connection();

        let run = foxhound
            .start_crawl(&declared.crawl, declared.seeds())
            .recording(Recording::replay(BOOKS_RECORDING))
            .await
            .expect("the crawl starts");
        let report = declared.crawl.serve(&run).await.expect("the crawl is served");

        assert_no_network(&report);
        let books = committed(&run, &declared.books).await;
        assert_eq!(books.len(), RECORDED_BOOKS);
        assert!(
            books.iter().all(|book| !book.category.is_empty()),
            "every book carries the category it was reached from"
        );
        run.as_run().close().await.expect("the Run closes");
        engine.close().await.expect("the engine closes");
    }

    #[tokio::test]
    async fn ai_extraction_replays_what_the_earlier_run_learned_and_calls_no_model() {
        let declared = ai_in_crawl_handler::crawl().expect("the crawl declares");
        // No model is configured on the test engine, and `plan.frozen` forbids any call anyway.
        let engine = offline_engine().await;
        let foxhound = engine.connection();
        let frozen = Settings::new().set(plan::FROZEN, true);

        let run = foxhound
            .start_crawl(
                &declared.crawl,
                [goodfetch_foxhound::Seed::new(
                    "https://books.toscrape.com/",
                    &declared.home,
                    (),
                )],
            )
            .plan(LEARNED_PLAN)
            .recording(Recording::replay(AI_RECORDING))
            .settings(frozen)
            .await
            .expect("the crawl starts");
        let report = declared.crawl.serve(&run).await.expect("the crawl is served");

        assert_no_network(&report);
        assert_eq!(report.run.plan.status, PlanStatus::Frozen);
        assert_eq!(report.run.usage.model.calls, 0, "a model was called");
        let books = committed(&run, &declared.books).await;
        assert_eq!(books.len(), RECORDED_AI_BOOKS);
        run.as_run().close().await.expect("the Run closes");
        engine.close().await.expect("the engine closes");
    }

    #[tokio::test]
    async fn ai_extraction_from_a_scripted_model_reply_calls_only_the_script() {
        let fields = BookFields {
            title: "A Light in the Attic".to_owned(),
            price: "51.77".to_owned(),
            in_stock: 22,
            rating: 3,
        };
        let reply = serde_json::to_string(&fields).expect("the fields serialize");
        let scripted = ScriptedModel::new([ScriptedReply::Response(ModelResponse::text(
            reply,
            ReportedTokens::new(1_000, 40),
        ))]);
        // The model reads the page at the call, so the one scripted reply is all it needs. The
        // only provider is the script: a real one is not registered, so none can be called.
        let settings = Settings::new()
            .set(ai::MODEL, Some("scripted/book".into()))
            .set(extract::MODE, ExtractMode::Model);
        let engine = TestEngine::start(
            TestOptions::default()
                .settings(settings)
                .model_provider("scripted", scripted.clone()),
        )
        .await
        .expect("the test engine starts");
        let declared = ai_in_crawl_handler::crawl().expect("the crawl declares");

        let input = HandlerInput::new(BOOK_URL, ()).response(SavedResponse::path(BOOK_PAGE));
        let attempt = engine
            .run_handler(&declared.crawl, &declared.book, input)
            .await
            .unwrap_or_else(|error| panic!("the handler did not run: {}", address_tried(&error)));

        assert!(
            matches!(attempt.ended, AttemptEnd::Returned),
            "the handler ended {:?}",
            attempt.ended
        );
        assert_eq!(attempt.usage.calls, 1);
        assert_eq!(scripted.requests().len(), 1);
        assert_eq!(scripted.remaining(), 0);
        let books = attempt.records(&declared.books).expect("the Records read");
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].fields, fields);
        assert_eq!(books[0].url, BOOK_URL);
        // Closing fails with `scripted_model_unused` if a reply was left, and nothing else.
        engine.close().await.expect("the engine closes");
    }

    #[tokio::test]
    async fn a_request_the_recording_lacks_fails_naming_its_address() {
        let declared = two_page_crawl::books_crawl().expect("the crawl declares");
        let engine = offline_engine().await;

        // The handler's document is not given and no recording backs it, so the engine refuses it.
        let input = HandlerInput::new(
            BOOK_URL,
            BookPayload {
                category: "Poetry".to_owned(),
            },
        );
        let refused = engine
            .run_handler(&declared.crawl, &declared.book, input)
            .await
            .expect_err("a document nobody recorded must be refused");

        assert_eq!(refused.code().as_str(), "response_not_recorded");
        assert!(
            address_tried(&refused).contains(BOOK_URL),
            "the failure must name the address: {}",
            address_tried(&refused)
        );

        // A whole crawl with no recording replays an empty one: the seed is refused, and the report
        // lists it.
        let run = engine
            .connection()
            .start_crawl(&declared.crawl, declared.seeds())
            .await
            .expect("the crawl starts");
        let report = declared.crawl.serve(&run).await.expect("the crawl is served");
        let recording = report.run.recording.as_ref().expect("the Run replays");
        assert!(
            recording
                .misses
                .iter()
                .any(|miss| miss.url == "https://books.toscrape.com/"),
            "the report must list the seed's address: {:?}",
            recording.misses
        );
        run.as_run().close().await.expect("the Run closes");
        engine.close().await.expect("the engine closes");
    }
}
