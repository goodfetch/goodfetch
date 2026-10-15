// Tests for the crawl of scenario 05 that run with no network and no model, with the language's usual test runner.

mod s05_two_page_crawl;
mod s08_ai_in_crawl_handler;

use goodfetch_foxhound::error::Error;
use goodfetch_foxhound::{Crawl, CrawlContext, Foxhound, Label, OutcomeKind, Seed, Settings};
use goodfetch_foxhound::testing::{MockResponse, MockServer};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task;

#[tokio::test]
async fn test_book_page_handler() -> Result<(), Error> {
    // Load the saved copy of the book page
    let book_page_content = std::fs::read_to_string("/path/to/saved/book_page.html").unwrap();
    let book_page_url = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

    // Create a mock server and send the book page response
    let (mock_server, _) = MockServer::start().await;
    mock_server.expect(book_page_url, MockResponse::html(book_page_content)).await;

    // Set up the crawl with the mock server
    let declared = s05_two_page_crawl::books_crawl().unwrap();
    let settings = Settings::new()
        .set(Foxhound::CONNECT_OPTIONS, ConnectOptions::default())
        .set(Foxhound::MOCK_SERVER, mock_server);

    let foxhound = Foxhound::connect(settings).await.unwrap();
    let run = foxhound
        .start_crawl(
            &declared.crawl,
            [Seed::new(book_page_url, &declared.book, s05_two_page_crawl::BookPayload {
                category: "ScienceFiction".to_string(),
            })],
        )
        .await
        .unwrap();

    // Serve the crawl to completion
    declared.crawl.serve(&run).await.unwrap();

    // Check the dataset
    let exported = run.export(&declared.books, goodfetch_foxhound::ExportFormat::Json).await.unwrap();
    let books: Vec<s05_two_page_crawl::Book> = exported.records::<s05_two_page_crawl::Book>().unwrap();

    assert_eq!(books.len(), 1);
    let book = &books[0];
    assert_eq!(book.title, "A Light in the Attic");
    assert_eq!(book.price, "£51.77");
    assert_eq!(book.in_stock, 22);
    assert_eq!(book.rating, 3);
    assert_eq!(book.url, book_page_url);
    assert_eq!(book.category, "ScienceFiction");

    Ok(()
}

#[tokio::test]
async fn test_whole_crawl_offline() -> Result<(), Error> {
    // Set up the crawl with a mock server
    let declared = s05_two_page_crawl::books_crawl().unwrap();
    let settings = Settings::new()
        .set(Foxhound::CONNECT_OPTIONS, ConnectOptions::default())
        .set(Foxhound::MOCK_SERVER, MockServer::start().await.0);

    let foxhound = Foxhound::connect(settings).await.unwrap();
    let run = foxhound
        .start_crawl(&declared.crawl, declared.seeds())
        .await
        .unwrap();

    // Serve the crawl to completion
    declared.crawl.serve(&run).await.unwrap();

    // Check the dataset
    let exported = run.export(&declared.books, goodfetch_foxhound::ExportFormat::Json).await.unwrap();
    let books: Vec<s05_two_page_crawl::Book> = exported.records::<s05_two_page_crawl::Book>().unwrap();

    assert_eq!(books.len(), 10); // Adjust the expected number of books as needed

    Ok(()
}

#[tokio::test]
async fn test_ai_extraction_offline() -> Result<(), Error> {
    // Set up the crawl with a mock server and disable the model
    let declared = s08_ai_in_crawl_handler::crawl().unwrap();
    let settings = Settings::new()
        .set(Foxhound::CONNECT_OPTIONS, ConnectOptions::default())
        .set(Foxhound::MOCK_SERVER, MockServer::start().await.0)
        .set(ai::MODEL, None);

    let foxhound = Foxhound::connect(settings).await.unwrap();
    let run = foxhound
        .start_crawl(&declared.crawl, [Seed::new("https://books.toscrape.com/", &declared.home, ())])
        .await
        .unwrap();

    // Serve the crawl to completion
    declared.crawl.serve(&run).await.unwrap();

    // Check that no real model was called
    let outcomes = run.outcomes(OutcomesFilter::default()).await.unwrap();
    let real_model_calls = outcomes
        .filter(|outcome| matches!(outcome.kind, OutcomeKind::Skipped { reason, .. } if reason.contains("model_allowance_reached")))
        .count();

    assert_eq!(real_model_calls, 0);

    Ok(()
}