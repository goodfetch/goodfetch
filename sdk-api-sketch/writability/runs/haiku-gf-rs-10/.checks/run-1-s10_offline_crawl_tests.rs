//! Scenario 10: offline tests of scenario 05's crawl and scenario 08's AI extraction.
//!
//! Nothing here reaches the network or a model. The tests run through `goodfetch_foxhound::testing`:
//! a handler runs on a saved copy of a page, a crawl replays the Response recording of one earlier
//! live run, and the AI extraction answers from a Scripted model. A request the recording lacks ends
//! `failed` with `response_not_recorded`, naming the address it tried, and a model the test does not
//! script is never reached, so any call to one fails the test.
//!
//! Run with `cargo test --example s10_offline_crawl_tests`.

// The scenarios' own `main` and `run` are for their binaries; these tests use their declarations.
#[expect(dead_code, reason = "the scenario's own `main` and `run` are not used by these tests")]
#[path = "s05_two_page_crawl.rs"]
mod two_page_crawl;

#[expect(dead_code, reason = "the scenario's own `main` is not used by these tests")]
#[path = "s08_ai_in_crawl_handler.rs"]
mod ai_in_crawl_handler;

#[cfg(not(test))]
fn main() {}

#[cfg(test)]
mod tests {
    use goodfetch_foxhound::settings::{Settings, ai};
    use goodfetch_foxhound::testing::{
        AttemptEnd, HandlerInput, SavedResponse, ScriptedModel, ScriptedReply, TestEngine,
        TestOptions,
    };
    use goodfetch_foxhound::{Error, ModelResponse, Recording, ReportedTokens};

    use super::ai_in_crawl_handler;
    use super::two_page_crawl::{BookPayload, books_crawl};

    /// The page's own address, as the crawl reaches it.
    const BOOK_URL: &str =
        "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

    /// A copy of that book's product page, as a browser saved its markup.
    const BOOK_PAGE: &str = r##"<!DOCTYPE html>
<html lang="en-us">
<head>
<meta charset="utf-8">
<title>A Light in the Attic | Books to Scrape - Sandbox</title>
</head>
<body>
<div class="container-fluid page">
<div class="page_inner">
<ul class="breadcrumb">
<li><a href="../../../index.html">Home</a></li>
<li><a href="../../category/books/poetry_23/index.html">Poetry</a></li>
<li class="active">A Light in the Attic</li>
</ul>
<div class="row">
<div class="col-sm-6 product_main">
<h1>A Light in the Attic</h1>
<p class="price_color">£51.77</p>
<p class="instock availability">
<i class="icon-ok"></i>
In stock (22 available)
</p>
<p class="star-rating Three">
<i class="icon-star"></i>
<i class="icon-star"></i>
<i class="icon-star"></i>
<i class="icon-star"></i>
<i class="icon-star"></i>
</p>
</div>
</div>
</div>
</div>
</body>
</html>
"##;

    /// The Response recording the earlier live run of scenario 05 wrote with `Recording::record`.
    const RECORDING: &str = "recordings/books_crawl.warc";

    /// The books on books.toscrape.com, across its 50 categories.
    const BOOKS_IN_SITE: usize = 1000;

    /// The book page as a saved document, the way a handler's test is fed one.
    fn saved_book_page() -> SavedResponse {
        SavedResponse::body(BOOK_PAGE.as_bytes().to_vec())
            .header("content-type", "text/html; charset=utf-8")
    }

    /// Panics unless the attempt returned. Any other end names what the handler did instead; a
    /// `response_not_recorded` failure names the address the handler tried to reach.
    fn assert_returned(ended: &AttemptEnd) {
        match ended {
            AttemptEnd::Returned => {}
            other => panic!("the handler did not return: {other:?}"),
        }
    }

    #[tokio::test]
    async fn book_page_handler_yields_the_book_with_its_category() -> Result<(), Error> {
        let declared = books_crawl()?;
        let engine = TestEngine::start(TestOptions::default()).await?;
        let input = HandlerInput::new(
            BOOK_URL,
            BookPayload {
                category: "Poetry".to_owned(),
            },
        )
        .response(saved_book_page());

        let attempt = engine
            .run_handler(&declared.crawl, &declared.book, input)
            .await?;
        assert_returned(&attempt.ended);

        let books = attempt.records(&declared.books)?;
        assert_eq!(books.len(), 1, "the page yields one book");
        let book = &books[0];
        assert_eq!(book.title, "A Light in the Attic");
        assert_eq!(book.price, "51.77");
        assert_eq!(book.in_stock, 22);
        assert_eq!(book.rating, 3);
        assert_eq!(book.url, BOOK_URL);
        assert_eq!(book.category, "Poetry", "the category the request carried");
        engine.close().await
    }

    #[tokio::test]
    async fn whole_crawl_from_the_recorded_run_holds_every_book() -> Result<(), Error> {
        let declared = books_crawl()?;
        let engine = TestEngine::start(TestOptions::default()).await?;
        let run = engine
            .connection()
            .start_crawl(&declared.crawl, declared.seeds())
            .recording(Recording::replay(RECORDING))
            .await?;
        let report = declared.crawl.serve(&run).await?;

        // A request the recording lacks is refused, never sent; the report lists each one.
        if let Some(recording) = &report.run.recording {
            let asked: Vec<&str> = recording
                .misses
                .iter()
                .map(|miss| miss.url.as_str())
                .collect();
            assert!(
                asked.is_empty(),
                "the crawl asked for addresses the recording lacks, which it would have reached: {asked:?}"
            );
        }

        let mut records = run.records(&declared.books);
        let mut count = 0;
        while let Some(record) = records.next().await {
            record?;
            count += 1;
        }
        assert_eq!(
            count, BOOKS_IN_SITE,
            "the dataset holds the books of the site"
        );
        engine.close().await
    }

    #[tokio::test]
    async fn ai_extraction_runs_from_a_scripted_reply_with_no_real_model() -> Result<(), Error> {
        let declared = ai_in_crawl_handler::crawl()?;
        // The only model is this one, and it answers exactly one call. A real provider is not
        // registered, so no other model can answer.
        let model = ScriptedModel::new([ScriptedReply::Response(ModelResponse::text(
            r#"{"title":"A Light in the Attic","price":"51.77","in_stock":22,"rating":3}"#
                .to_owned(),
            ReportedTokens::new(1200, 80),
        ))]);
        let options = TestOptions::default()
            .settings(Settings::new().set(ai::MODEL, Some("vercel/openai/gpt-5-mini".into())))
            .model_provider("vercel", model.clone());
        let engine = TestEngine::start(options).await?;
        let input = HandlerInput::new(BOOK_URL, ()).response(saved_book_page());

        let attempt = engine
            .run_handler(&declared.crawl, &declared.book, input)
            .await?;
        assert_returned(&attempt.ended);
        assert_eq!(
            model.requests().len(),
            1,
            "the extraction made exactly one model call"
        );

        let books = attempt.records(&declared.books)?;
        assert_eq!(books.len(), 1, "the extraction yields one book");
        let book = &books[0];
        assert_eq!(book.fields.title, "A Light in the Attic");
        assert_eq!(book.fields.price, "51.77");
        assert_eq!(book.fields.in_stock, 22);
        assert_eq!(book.fields.rating, 3);
        assert_eq!(book.url, BOOK_URL);
        engine.close().await
    }
}
