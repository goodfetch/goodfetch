//! A small web service with one endpoint, `GET /books?category=<name>`. When called, it crawls that
//! category of https://books.toscrape.com/ (following its "next" pages) and answers with its books
//! as JSON, each with its title and price.
//!
//! Every call opens its own Foxhound connection and starts its own Run, with no fixed Run ID, so
//! calls running at the same time, or later, do not interfere. The connection is lent to the crawl
//! through `Foxhound::scope`, which closes it on every return and error. When the caller disconnects,
//! axum drops the request's future; that drops the connection, which cancels the crawl and releases
//! the connections and browsers it opened.
//!
//! The library emits `tracing` events under `goodfetch::` targets and never installs a subscriber.
//! To send them to the service's logger, install a `tracing` subscriber in `main`; this example has
//! no tracing dependency, so it installs none, and nothing is printed to standard output.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::routing::get;
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, Dataset, Error, Foxhound, Label, LinkInfo, Seed,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The site's home page, where the crawl starts to find the category.
const HOME: &str = "https://books.toscrape.com/";
/// The sidebar links to every category, on the home page.
const CATEGORY_LINKS: &str = "div.side_categories ul li ul li a";
/// One product on a category page.
const PRODUCTS: &str = "article.product_pod";
/// The address the service listens on.
const BIND: &str = "127.0.0.1:3000";
/// How long one crawl may last.
const RUN_TIMEOUT: Duration = Duration::from_secs(600);

/// One book of a category.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Book {
    /// The full title, from the product's link.
    pub title: String,
    /// The price as the page shows it, such as "£51.77".
    pub price: String,
}

/// The query of `GET /books`.
#[derive(Debug, Deserialize)]
struct BooksQuery {
    /// The category's name as the site's sidebar shows it, such as "Mystery".
    category: String,
}

/// Crawls the category named `category` and returns its books, or `None` when the site has no
/// category of that name.
async fn crawl_category(category: &str) -> Result<Option<Vec<Book>>, Error> {
    Foxhound::scope(ConnectOptions::default().no_project(), async |fox: &Foxhound| -> Result<Option<Vec<Book>>, Error> {
        let found = AtomicBool::new(false);
        let mut crawl = Crawl::new();
        let books: Dataset<Book> = crawl.dataset("books")?;
        let home: Label<()> = crawl.label("home")?;
        let category_label: Label<()> = crawl.label("category")?;

        // The home page enqueues the sidebar link whose text is the requested category name.
        crawl.handle(&home, async |ctx: CrawlContext<()>| -> Result<(), Error> {
            let links = ctx.page().locator(CATEGORY_LINKS);
            let added = ctx
                .enqueue_links_with(&links, &category_label, |link: &LinkInfo| {
                    link.text.trim().eq_ignore_ascii_case(category).then_some(())
                })
                .await?;
            if !added.added.is_empty() {
                found.store(true, Ordering::Relaxed);
            }
            Ok(())
        })?;

        // A category page yields its books, then enqueues its "next" page under the same label.
        crawl.handle(&category_label, async |ctx: CrawlContext<()>| -> Result<(), Error> {
            let products = ctx.page().locator(PRODUCTS).all().await?;
            for product in products {
                let title = product
                    .locator("h3 a")
                    .get_attribute("title")
                    .await?
                    .unwrap_or_default();
                let price = product.locator(".price_color").text_content().await?;
                let book = Book {
                    title,
                    price: price.trim().to_owned(),
                };
                ctx.push(&books, &book).await?;
            }
            let next = ctx.page().locator("li.next a");
            ctx.enqueue_links(&next, &category_label, ()).await?;
            Ok(())
        })?;

        let run = fox
            .start_crawl(&crawl, [Seed::new(HOME, &home, ())])
            .timeout(RUN_TIMEOUT)
            .await?;
        let _report = crawl.serve(&run).await?;

        if !found.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut records = run.records(&books);
        let mut collected = Vec::new();
        while let Some(record) = records.next().await {
            collected.push(record?.value);
        }
        Ok(Some(collected))
    })
    .await
}

/// `GET /books?category=<name>`: the books of that category, as JSON.
async fn list_books(Query(query): Query<BooksQuery>) -> Result<Json<Vec<Book>>, (StatusCode, String)> {
    let category = query.category.trim();
    if category.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "the category parameter is empty".to_owned()));
    }
    match crawl_category(category).await {
        Ok(Some(books)) => Ok(Json(books)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("no category named {category}"))),
        Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())),
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let app = Router::new().route("/books", get(list_books));
    let listener = tokio::net::TcpListener::bind(BIND).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}
