//! Snippet for Testing a user's crawl or automation (#482, ADR 0105), on golden scenario 10: the
//! crawl of scenario 05 and the AI extraction of scenario 08, tested with no network and no model.
//! In a program these are `#[tokio::test]` functions, or `#[test]` with `goodfetch_foxhound::blocking`; the
//! kit needs no harness of its own. It is behind the `test-util` feature, a dev-dependency.

use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::settings::{ai, extract, plan};
use goodfetch_foxhound::testing::{AttemptEnd, HandlerInput, SavedResponse, ScriptedModel, ScriptedReply, TestEngine, TestOptions};
use goodfetch_foxhound::{BoxError, Crawl, CrawlContext, Dataset, DatasetDeclaration, Error, ErrorCode, ExtractMode, Label, ModelReference, Recording, Schema, Seed, Settings};
use serde::{Deserialize, Serialize};

const BOOK: &str = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

/// Scenario 05's book.
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Book {
    title: String,
    price: String,
    url: String,
    category: String,
}

/// The Payload of a book request.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookPayload {
    category: String,
}

/// The book handler of scenario 05, as the program writes it.
async fn on_book(books: &Dataset<Book>, ctx: CrawlContext<BookPayload>) -> Result<(), BoxError> {
    let main = ctx.page().locator(".product_main");
    let price = main.locator(".price_color").text_content().await?;
    let Some(pounds) = price.strip_prefix('£') else {
        return Err(ctx.skip("no price"));
    };
    let book = Book {
        title: main.locator("h1").text_content().await?,
        price: pounds.to_owned(),
        url: ctx.page().url(),
        category: ctx.request().payload().category.clone(),
    };
    ctx.push(books, &book).await?;
    Ok(())
}

/// 1. One handler, one saved page: no network, no browser, no Frontier.
async fn book_page_gives_its_book() -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;
    let book = crawl.label::<BookPayload>("book")?;
    crawl.handle(&book, async |ctx: CrawlContext<BookPayload>| on_book(&books, ctx).await)?;
    let engine = TestEngine::start(TestOptions::default()).await?;
    let attempt = engine
        .run_handler(
            &crawl,
            &book,
            HandlerInput::new(BOOK, BookPayload { category: "Poetry".into() })
                .response(SavedResponse::path("tests/fixtures/a-light-in-the-attic.html")),
        )
        .await?;
    assert!(matches!(attempt.ended, AttemptEnd::Returned));
    assert_eq!(
        attempt.records(&books)?,
        [Book { title: "A Light in the Attic".into(), price: "51.77".into(), url: BOOK.into(), category: "Poetry".into() }]
    );
    engine.close().await
}

/// 2. The whole crawl, from the responses a live run recorded with `Recording::record(..)`.
async fn crawl_collects_every_book_from_the_recording() -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset(DatasetDeclaration::<Book>::new("books").key(&["url"]))?;
    let home = crawl.label::<()>("home")?;
    let book = crawl.label::<BookPayload>("book")?;
    crawl.handle(&home, async |ctx: CrawlContext<()>| {
        let links = ctx.page().locator(".side_categories ul ul a");
        ctx.enqueue_links(&links, &book, BookPayload { category: String::new() }).await?;
        Ok(())
    })?;
    crawl.handle(&book, async |ctx: CrawlContext<BookPayload>| on_book(&books, ctx).await)?;
    let engine = TestEngine::start(TestOptions::default()).await?;
    let run = engine
        .connection()
        .start_crawl(&crawl, [Seed::new("https://books.toscrape.com/", &home, ())])
        .recording(Recording::replay("tests/books.warc.gz"))
        .await?;
    let report = crawl.serve(&run).await?; // no Politeness waits: no request reaches the site
    assert_eq!(report.requests.failed, 0);
    let mut records = run.records(&books);
    let mut n = 0;
    while let Some(record) = records.next().await {
        record?;
        n += 1;
    }
    assert_eq!(n, 1000);
    // The recording's age is in the report, never a reason to fail.
    assert!(report.run.recording.is_some_and(|r| r.misses.is_empty()));
    engine.close().await
}

/// A page the recording lacks is never fetched: the error names its address.
async fn page_the_recording_lacks_fails() -> Result<(), Error> {
    let engine = TestEngine::start(TestOptions::default()).await?;
    let run = engine.connection().start().recording(Recording::replay("tests/books.warc.gz")).await?;
    let page = run.new_page().await?;
    let error = page.goto("https://books.toscrape.com/catalogue/page-99.html").await.expect_err("not recorded");
    assert_eq!(error.code(), ErrorCode::RESPONSE_NOT_RECORDED);
    assert!(matches!(error.details::<goodfetch_foxhound::ResponseNotRecorded>(), Some(d) if d.url.ends_with("page-99.html")));
    engine.close().await
}

/// The fields scenario 08 extracts with a model.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookFields {
    title: String,
    price: String,
}

/// Scenario 08's AI handler.
fn ai_crawl<'a>() -> Result<(Crawl<'a>, Label<()>), Error> {
    let mut crawl = Crawl::new();
    let book = crawl.label::<()>("book")?;
    crawl.handle(&book, async |ctx: CrawlContext<()>| {
        let _fields = ctx.page().extract("the book's title and price", Schema::<BookFields>::of()).await?.output;
        Ok(())
    })?;
    Ok((crawl, book))
}

/// 3. The AI extraction replays the Plan learned on the live run: frozen, it calls no model.
async fn ai_extraction_replays_its_plan() -> Result<(), Error> {
    let (crawl, book) = ai_crawl()?;
    let engine = TestEngine::start(TestOptions::default()).await?; // no model registered
    let run = engine
        .connection()
        .start_crawl(&crawl, [Seed::new(BOOK, &book, ())])
        .plan("tests/books.plan.json")
        .recording(Recording::replay("tests/books.warc.gz"))
        .settings(Settings::new().set(plan::FROZEN, true))
        .await?;
    let report = crawl.serve(&run).await?;
    assert_eq!(report.run.usage.model.calls, 0);
    engine.close().await
}

/// Or a Scripted model that answers the extraction in its active mode, here the default compiled
/// one: the kit writes the locators that read these values from the saved page, and the step reads
/// the page with them as in production.
async fn ai_extraction_compiles_from_scripted_data() -> Result<(), Error> {
    let (crawl, book) = ai_crawl()?;
    let data = serde_json::json!({ "title": "A Light in the Attic", "price": "£51.77" });
    let model = ScriptedModel::new([ScriptedReply::Extraction(data)]);
    let settings = Settings::new().set(ai::MODEL, ModelReference::from("scripted/any"));
    let engine = TestEngine::start(TestOptions::default().model_provider("scripted", model.clone()).settings(settings)).await?;
    let attempt = engine
        .run_handler(&crawl, &book, HandlerInput::new(BOOK, ()).response(SavedResponse::path("tests/fixtures/a-light-in-the-attic.html")))
        .await?;
    // A value not on the page would fail the step with `extraction_incomplete`, naming the field.
    assert!(matches!(attempt.ended, AttemptEnd::Returned));
    assert_eq!(model.requests().len(), 1);
    engine.close().await
}

/// Or a Scripted model: a provider whose funds are spent, an error no retry helps.
async fn funds_spent() -> Result<(), Error> {
    let (crawl, book) = ai_crawl()?;
    let model = ScriptedModel::new([ScriptedReply::Error(Error::fake(ErrorCode::MODEL_FUNDS_EXHAUSTED, "credit balance too low"))]);
    let settings = Settings::new().set(ai::MODEL, ModelReference::from("scripted/any")).set(extract::MODE, ExtractMode::Model);
    let engine = TestEngine::start(TestOptions::default().model_provider("scripted", model.clone()).settings(settings)).await?;
    let attempt = engine
        .run_handler(&crawl, &book, HandlerInput::new(BOOK, ()).response(SavedResponse::path("tests/fixtures/a-light-in-the-attic.html")))
        .await?;
    assert!(matches!(&attempt.ended, AttemptEnd::Failed(e) if e.code() == ErrorCode::MODEL_FUNDS_EXHAUSTED));
    assert_eq!(model.requests().len(), 1);
    engine.close().await // fails if a reply is left unused
}

fn main() {
    let _ = (book_page_gives_its_book, crawl_collects_every_book_from_the_recording, page_the_recording_lacks_fails);
    let _ = (ai_extraction_replays_its_plan, ai_extraction_compiles_from_scripted_data, funds_spent);
    let _ = schemars::schema_for!(Book);
}
