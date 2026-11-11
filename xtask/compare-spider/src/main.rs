//! One crawl with Spider, for `just compare` (ADRs 0134 and 0138's matrix):
//! from `--url`, at `--concurrency` requests at once, every page of its
//! origin, with `--host`, when given, sent as each request's `Host` so the
//! Proving ground serves its family on the loopback address. `--scenario`
//! names the matrix's scenario: `extraction` also writes each page's URL,
//! title and HTML as one JSON line to `pages.jsonl` in the working folder,
//! and `browser` renders each page in Chrome (`CHROME_BIN`), which only the
//! executable built with the `chrome` feature does, or, built with the
//! `smart` feature, fetches over HTTP first and renders where a page needs
//! it (`crawl_smart`); every other scenario crawls in Spider's HTTP mode.
//!
//! `--capacity` is the capacity of the broadcast channel Spider sends each
//! page on. Spider's `Website` keeps a receiver of its own that never
//! reads, and Tokio frees a slot only once every receiver has read it, so
//! the channel keeps that many whole pages alive: the capacity is part of
//! Spider's memory, and the comparison's matrix chooses it (#2213). Each
//! page is read as it arrives, and a page the channel dropped before it was
//! read (`Lagged`) fails the crawl, so a crawl that ends well lost none.
//!
//! The `jemalloc` and `mimalloc` features set the global allocator Spider's
//! README recommends; without either, the system's serves.
//!
//! It prints one line, `pages=<n> body_bytes=<n>`, what Spider says it
//! fetched; the comparison's same-work check reads the Proving ground's own
//! observations instead.
//!
//!   goodfetch-compare-spider --url http://127.0.0.1:<port>/sites/1000/1 \
//!     --host scale.test:<port> --concurrency 16 --scenario raw --capacity 64

use std::io::Write as _;
use std::process::ExitCode;

use spider::reqwest::header::{HOST, HeaderMap, HeaderValue};
use spider::website::Website;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The file the `extraction` scenario writes each page to.
const PAGES_FILE: &str = "pages.jsonl";

/// What one crawl fetched, as Spider reports it, and where each page is
/// written, if anywhere.
#[derive(Default)]
struct Fetched {
    pages: u64,
    body_bytes: u64,
    written: Option<std::io::BufWriter<std::fs::File>>,
}

impl Fetched {
    fn add(&mut self, page: &spider::page::Page) -> Result<(), String> {
        self.pages += 1;
        self.body_bytes += page.get_bytes().map_or(0, |bytes| bytes.len() as u64);
        if let Some(written) = self.written.as_mut() {
            let title = page
                .get_metadata()
                .as_ref()
                .and_then(|metadata| metadata.title.as_ref())
                .map(|title| title.to_string());
            let line = serde_json::json!({
                "url": page.get_url(),
                "title": title,
                "content": page.get_html(),
            });
            writeln!(written, "{line}").map_err(|error| format!("writing {PAGES_FILE}: {error}"))?;
        }
        Ok(())
    }
}

impl Fetched {
    /// Flushes the pages written, once the stream is drained.
    fn finish(mut self) -> Result<Self, String> {
        if let Some(written) = self.written.as_mut() {
            written.flush().map_err(|error| format!("writing {PAGES_FILE}: {error}"))?;
        }
        Ok(self)
    }
}

struct Arguments {
    url: String,
    host: Option<String>,
    concurrency: usize,
    scenario: String,
    /// The page channel's capacity, at least 1: Spider reads 0 as its own
    /// default.
    capacity: usize,
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: goodfetch-compare-spider --url <url> [--host <host:port>] --concurrency <n> --scenario <name> --capacity <n>";
    let mut url = None;
    let mut host = None;
    let mut concurrency = None;
    let mut scenario = None;
    let mut capacity = None;
    let mut given = std::env::args().skip(1);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(usage)?;
        match name.as_str() {
            "--url" => url = Some(value),
            "--host" => host = Some(value),
            "--concurrency" => {
                concurrency = Some(value.parse().map_err(|_| "--concurrency takes a count")?);
            }
            "--scenario" => scenario = Some(value),
            "--capacity" => {
                capacity = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|capacity| *capacity > 0)
                        .ok_or("--capacity takes a count of at least 1")?,
                );
            }
            _ => return Err(usage.to_owned()),
        }
    }
    Ok(Arguments {
        url: url.ok_or(usage)?,
        host,
        concurrency: concurrency.ok_or(usage)?,
        scenario: scenario.ok_or(usage)?,
        capacity: capacity.ok_or(usage)?,
    })
}

async fn crawl(arguments: &Arguments) -> Result<Fetched, String> {
    let browser = match arguments.scenario.as_str() {
        "raw" | "extraction" => false,
        "browser" if cfg!(feature = "chrome") => true,
        "browser" => return Err("the browser scenario takes the `chrome` feature's executable".to_owned()),
        other => return Err(format!("Spider runs no `{other}` scenario")),
    };
    let mut site = Website::new(&arguments.url);
    site.with_concurrency_limit(Some(arguments.concurrency))
        .with_respect_robots_txt(false);
    if let Some(host) = &arguments.host {
        let mut headers = HeaderMap::new();
        headers.insert(
            HOST,
            HeaderValue::from_str(host).map_err(|error| error.to_string())?,
        );
        site.with_headers(Some(headers));
    }
    let written = if arguments.scenario == "extraction" {
        Some(std::io::BufWriter::new(
            std::fs::File::create(PAGES_FILE).map_err(|error| format!("creating {PAGES_FILE}: {error}"))?,
        ))
    } else {
        None
    };
    // Every page is read from the stream, drained once the crawl ends:
    // the lab found the crawl's end may come before its last page is read.
    // The reader is a task of its own, as Spider's documentation subscribes
    // (`Website::subscribe`'s example): beside the crawl in one task, it
    // read only when the crawl's own future yielded, and under Callgrind a
    // channel of 64 or of 128 pages lost some (#2213).
    let mut pages = site.subscribe(arguments.capacity);
    let (done, mut ended) = tokio::sync::oneshot::channel::<()>();
    let reading = tokio::spawn(async move {
        let mut fetched = Fetched { written, ..Fetched::default() };
        loop {
            tokio::select! {
                page = pages.recv() => match page {
                    Ok(page) => fetched.add(&page)?,
                    Err(RecvError::Lagged(lost)) => return Err(format!("the page stream lost {lost} pages")),
                    Err(RecvError::Closed) => return fetched.finish(),
                },
                _ = &mut ended => loop {
                    match pages.try_recv() {
                        Ok(page) => fetched.add(&page)?,
                        Err(TryRecvError::Lagged(lost)) => return Err(format!("the page stream lost {lost} pages")),
                        Err(_) => return fetched.finish(),
                    }
                },
            }
        }
    });
    if browser && cfg!(feature = "smart") {
        site.crawl_smart().await;
    } else if browser {
        site.crawl().await;
    } else {
        site.crawl_raw().await;
    }
    // The receiver lives until the stream is drained.
    let _ = done.send(());
    reading
        .await
        .map_err(|error| format!("reading the page stream: {error}"))?
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
