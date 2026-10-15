//! Golden scenario 12, "Run a crawl inside a web service" (Foxhound, Rust).
//!
//! `cargo run --example s12_crawl_in_a_service` serves `/books?category=<name>` on port 8000;
//! `cargo run --example s12_crawl_in_a_service -- Travel` runs the same crawl as a plain script.
//! Foxhound logs through `tracing`, so the service's own subscriber receives its events, and with
//! none installed nothing is written anywhere.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use goodfetch_foxhound::schemars::{self, JsonSchema};
use goodfetch_foxhound::{
    AriaRole, ConnectOptions, Crawl, CrawlContext, Error, Foxhound, Seed, SkipReason,
};
use serde::{Deserialize, Serialize};

/// A book of the category, as the endpoint answers it.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Book {
    /// A missing title attribute fails this bound: the request fails with `record_invalid`.
    #[schemars(length(min = 1))]
    title: String,
    price: String,
}

/// The category asked for, carried by the home page's request.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Category {
    name: String,
}

/// The endpoint's query string.
#[derive(Debug, Deserialize)]
struct BooksQuery {
    category: String,
}

/// The category's books, or `None` when the site has no such category. Each call declares its
/// crawl and starts a Run of its own, so calls running at once share no state. Dropping this
/// future drops `serve`, the program's own stop, then the unclosed Run, whose `Drop` stops it
/// without blocking and releases its browser and connections (ADRs 0078, 0093).
async fn books_in(foxhound: &Foxhound, name: &str) -> Result<Option<Vec<Book>>, Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset::<Book>("books")?;
    let home = crawl.label::<Category>("home")?;
    let category = crawl.label::<()>("category")?;

    crawl.handle(&home, async |ctx: CrawlContext<Category>| {
        let name = &ctx.request().payload().name;
        let link = ctx
            .page()
            .locator(".side_categories ul ul")
            .get_by_role(AriaRole::Link.name(name.as_str()).exact());
        let enqueued = ctx.enqueue_links(&link, &category, ()).await?;
        if enqueued.added.is_empty() {
            return Err(ctx.skip(&format!("no category named {name:?}")));
        }
        Ok(())
    })?;

    crawl.handle(&category, async |ctx: CrawlContext<()>| {
        for book in ctx.page().locator("article.product_pod").all().await? {
            let title = book.locator("h3 a").get_attribute("title").await?;
            let record = Book {
                title: title.unwrap_or_default(),
                price: book.locator(".price_color").text_content().await?,
            };
            ctx.push(&books, &record).await?;
        }
        let next = ctx.page().locator("li.next a");
        ctx.enqueue_links(&next, ctx.request().label(), ()).await?;
        Ok(())
    })?;

    let payload = Category {
        name: name.to_owned(),
    };
    let seed = Seed::new("https://books.toscrape.com/", &home, payload);
    let run = foxhound.start_crawl(&crawl, [seed]).await?;
    // Serving is what lets a crawl finish: it returns the Run's report once the Run ends (ADR 0102).
    let report = crawl.serve(&run).await?;
    let skips = &report.requests.by_skip_reason;
    let missing = skips
        .iter()
        .any(|(reason, _)| *reason == SkipReason::Handler);
    let mut found = Vec::new();
    let mut records = run.records(&books);
    while let Some(record) = records.next().await {
        found.push(record?.value);
    }
    run.as_run().close().await?;
    Ok((!missing).then_some(found))
}

/// `GET /books?category=<name>`. When the caller disconnects, axum drops this future.
async fn get_books(
    State(foxhound): State<Foxhound>,
    Query(query): Query<BooksQuery>,
) -> Result<Json<Vec<Book>>, (StatusCode, String)> {
    match books_in(&foxhound, &query.category).await {
        Ok(Some(found)) => Ok(Json(found)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            format!("no category named {:?}", query.category),
        )),
        Err(error) => Err((StatusCode::BAD_GATEWAY, error.to_string())),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One connection for the program's life; a handle is a cheap clone, shared by every call.
    let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    if let Some(name) = std::env::args().nth(1) {
        let found = books_in(&foxhound, &name).await?;
        println!("{found:?}"); // the script's own output
    } else {
        let app = Router::new()
            .route("/books", get(get_books))
            .with_state(foxhound.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
        // The service's own signal handling: Foxhound installs none.
        let stopped = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        axum::serve(listener, app)
            .with_graceful_shutdown(stopped)
            .await?;
    }
    foxhound.close().await?;
    Ok(())
}
