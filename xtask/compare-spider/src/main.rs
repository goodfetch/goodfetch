//! One crawl with Spider in its HTTP mode, for `just compare-spider` (ADR
//! 0134): from `--url`, at `--concurrency` requests at once, every page of
//! its origin, with `--host` sent as each request's `Host` so the Proving
//! ground serves its family on the loopback address. It prints one line,
//! `pages=<n> body_bytes=<n>`, what Spider says it fetched; the comparison's
//! same-work check reads the Proving ground's own observations instead.
//!
//!   goodfetch-compare-spider --url http://127.0.0.1:<port>/sites/1000/1 \
//!     --host scale.test:<port> --concurrency 16

use std::process::ExitCode;

use spider::reqwest::header::{HOST, HeaderMap, HeaderValue};
use spider::website::Website;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

/// What one crawl fetched, as Spider reports it.
#[derive(Default)]
struct Fetched {
    pages: u64,
    body_bytes: u64,
}

impl Fetched {
    fn add(&mut self, page: &spider::page::Page) {
        self.pages += 1;
        self.body_bytes += page.get_bytes().map_or(0, |bytes| bytes.len() as u64);
    }
}

struct Arguments {
    url: String,
    host: String,
    concurrency: usize,
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: goodfetch-compare-spider --url <url> --host <host:port> --concurrency <n>";
    let mut url = None;
    let mut host = None;
    let mut concurrency = None;
    let mut given = std::env::args().skip(1);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(usage)?;
        match name.as_str() {
            "--url" => url = Some(value),
            "--host" => host = Some(value),
            "--concurrency" => {
                concurrency = Some(value.parse().map_err(|_| "--concurrency takes a count")?);
            }
            _ => return Err(usage.to_owned()),
        }
    }
    Ok(Arguments {
        url: url.ok_or(usage)?,
        host: host.ok_or(usage)?,
        concurrency: concurrency.ok_or(usage)?,
    })
}

async fn crawl(arguments: &Arguments) -> Result<Fetched, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        HOST,
        HeaderValue::from_str(&arguments.host).map_err(|error| error.to_string())?,
    );
    let mut site = Website::new(&arguments.url);
    site.with_concurrency_limit(Some(arguments.concurrency))
        .with_respect_robots_txt(false)
        .with_headers(Some(headers));
    // Every page is read from the stream, drained once the crawl ends:
    // the lab found the crawl's end may come before its last page is read.
    let mut pages = site.subscribe(4096);
    let (done, mut ended) = tokio::sync::oneshot::channel::<()>();
    let crawling = async {
        site.crawl_raw().await;
        // The receiver lives until the stream is drained.
        let _ = done.send(());
    };
    let reading = async {
        let mut fetched = Fetched::default();
        loop {
            tokio::select! {
                page = pages.recv() => match page {
                    Ok(page) => fetched.add(&page),
                    Err(RecvError::Lagged(lost)) => return Err(format!("the page stream lost {lost} pages")),
                    Err(RecvError::Closed) => return Ok(fetched),
                },
                _ = &mut ended => loop {
                    match pages.try_recv() {
                        Ok(page) => fetched.add(&page),
                        Err(TryRecvError::Lagged(lost)) => return Err(format!("the page stream lost {lost} pages")),
                        Err(_) => return Ok(fetched),
                    }
                },
            }
        }
    };
    let ((), fetched) = tokio::join!(crawling, reading);
    fetched
}

#[tokio::main]
async fn main() -> ExitCode {
    let fetched = match arguments() {
        Ok(arguments) => crawl(&arguments).await,
        Err(usage) => Err(usage),
    };
    match fetched {
        Ok(fetched) => {
            println!("pages={} body_bytes={}", fetched.pages, fetched.body_bytes);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("goodfetch-compare-spider: {error}");
            ExitCode::FAILURE
        }
    }
}
