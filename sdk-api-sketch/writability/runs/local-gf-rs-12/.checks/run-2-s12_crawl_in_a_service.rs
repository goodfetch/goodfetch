// s12_crawl_in_a_service.rs

use axum::{routing::get, Router};
use futures_util::stream::StreamExt;
use goodfetch_collie::Collie;
use goodfetch_foxhound::{Crawl, CrawlResult, Foxhound};
use goodfetch_foxhound_client::{Client, ClientConfig};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use std::net::SocketAddr;

#[derive(Serialize, Deserialize)]
struct Book {
    title: String,
    price: String,
}

async fn crawl_books(category: &str) -> Vec<Book> {
    let client = Client::new(ClientConfig::default()).await.unwrap();
    let mut crawl = Crawl::new(&client);
    crawl.settings.category = category.to_string();

    let mut books = Vec::new();
    let mut page = 1;

    loop {
        crawl.settings.page = page;
        let result: CrawlResult = crawl.run().await.unwrap();
        if result.books.is_empty() {
            break;
        }
        books.extend(result.books);
        page += 1;
    }

    books
}

async fn books_handler(category: String) -> Vec<Book> {
    crawl_books(&category).await
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/books", get(books_handler));
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}