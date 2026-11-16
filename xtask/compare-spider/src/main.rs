//! One crawl with Spider, for `just compare` (ADRs 0134 and 0138's matrix):
//! from `--url`, at `--concurrency` requests at once, every page of its
//! origin, with `--host`, when given, sent as each request's `Host` so the
//! Proving ground serves its family on the loopback address. `--scenario`
//! names the matrix's scenario: `extraction` also writes each page's URL,
//! title and Main content as one JSON line to `pages.jsonl` in the working
//! folder, which only the executable built with the `extraction` feature
//! does (#2136): the Main content is `llm_readability`'s text of the page,
//! the readability step of `spider_transformations`, Spider's only path to
//! one, and the title is Spider's page metadata's, its character references
//! decoded and its white space collapsed here, inside the timed process, as
//! the other tools' parsers do theirs. `browser` renders each page in
//! Chrome (`CHROME_BIN`), which only the executable built with the `chrome`
//! feature does. `auto`, the rungs site's (#2411), crawls with
//! `crawl_smart()`, which fetches each page over HTTP and renders it in
//! that Chrome only where Spider finds it needs one, which only the
//! executable built with the `smart` feature does: it writes each page's
//! URL, whether Spider rendered it in Chrome, and the Records its HTML
//! holds, each `article.item`'s `p.id` and `h1.name` text, read with
//! `lol_html`, the parser Spider's smart mode reads pages with, as one JSON
//! line to `pages.jsonl`. Every other scenario crawls in Spider's HTTP
//! mode.
//! `hostile`, the hostile pages' pass at each tool's defaults (#2439), sets
//! neither the requests in flight nor robots.txt, leaving Spider's defaults.
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
//! `--scenario admission`, given first, crawls nothing: it times Spider's
//! admission of links alone (#2318), as `admission.rs` says.
//!
//!   goodfetch-compare-spider --url http://127.0.0.1:<port>/sites/1000/1 \
//!     --host scale.test:<port> --concurrency 16 --scenario raw --capacity 64

mod admission;

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

/// The file the `extraction` and `auto` scenarios write each page to.
const PAGES_FILE: &str = "pages.jsonl";

/// A page's line of `pages.jsonl`, as its scenario reads it.
type Line = Result<serde_json::Value, String>;

/// What one crawl fetched, as Spider reports it, and where each page is
/// written, if anywhere.
#[derive(Default)]
struct Fetched {
    pages: u64,
    body_bytes: u64,
    written: Option<Written>,
}

/// The `extraction` and `auto` scenarios' pages: the file they are written
/// to, how the scenario reads a page, and the tasks reading them. Each page
/// is read in a task of its own, as `Website::subscribe`'s example
/// processes a page, so the one reader of the stream keeps up with the
/// crawl: with `llm_readability` run in the reader itself, crawls of the
/// 10,000-page site lost pages at every capacity from 256 to 4,096 (#2136).
struct Written {
    file: std::io::BufWriter<std::fs::File>,
    read: fn(&spider::page::Page) -> Line,
    reading: tokio::task::JoinSet<Line>,
}

impl Written {
    /// Writes the line of a page whose task ended.
    fn write(&mut self, read: Result<Line, tokio::task::JoinError>) -> Result<(), String> {
        let line = read.map_err(|error| format!("reading a page: {error}"))??;
        writeln!(self.file, "{line}").map_err(|error| format!("writing {PAGES_FILE}: {error}"))
    }
}

impl Fetched {
    fn add(&mut self, page: spider::page::Page) -> Result<(), String> {
        self.pages += 1;
        self.body_bytes += page.get_bytes().map_or(0, |bytes| bytes.len() as u64);
        if let Some(written) = self.written.as_mut() {
            let read = written.read;
            written.reading.spawn(async move { read(&page) });
            // The pages read so far are written as the stream goes.
            while let Some(read) = written.reading.try_join_next() {
                written.write(read)?;
            }
        }
        Ok(())
    }
}

/// White space collapsed to single spaces, Unicode's included, and trimmed.
#[cfg(any(feature = "extraction", feature = "smart"))]
fn collapsed(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    collapsed
}

/// A page's line of `pages.jsonl`: its URL, its title and its Main content.
/// The title is Spider's page metadata's, which holds the `<title>`'s source
/// text: its character references are decoded and its white space
/// collapsed. The Main content is `llm_readability`'s text of the page,
/// called as `spider_transformations` calls it, without the reader page it
/// wraps that text in, whose head repeats the title.
#[cfg(feature = "extraction")]
fn extracted(page: &spider::page::Page) -> Line {
    let title = page
        .get_metadata()
        .as_ref()
        .and_then(|metadata| metadata.title.as_ref())
        .map(|title| collapsed(&html_escape::decode_html_entities(title.as_str())));
    // The page's own URL, which a page read from the subscription has not
    // parsed yet; `spider_transformations` passes a placeholder there.
    let parsed;
    let url = match page.get_url_parsed_ref() {
        Some(url) => url,
        None => {
            parsed = spider::reqwest::Url::parse(page.get_url())
                .map_err(|error| format!("parsing {}: {error}", page.get_url()))?;
            &parsed
        }
    };
    let text = llm_readability::extractor::extract(&mut page.get_html_bytes_u8(), url)
        .map_err(|error| format!("reading {}'s main content: {error}", page.get_url()))?
        .text;
    Ok(serde_json::json!({ "url": page.get_url(), "title": title, "text": text }))
}

#[cfg(not(feature = "extraction"))]
fn extracted(_page: &spider::page::Page) -> Line {
    Err("the extraction scenario takes the `extraction` feature's executable".to_owned())
}

/// A page's line of `pages.jsonl` in the `auto` scenario: its URL, whether
/// Spider rendered it in Chrome, which only its Chrome path records the
/// bytes transferred of, and the Records its HTML holds: each
/// `article.item`'s `p.id` and `h1.name` text, white space collapsed, one
/// holding neither none.
#[cfg(feature = "smart")]
fn records(page: &spider::page::Page) -> Line {
    use std::cell::RefCell;

    use lol_html::{HtmlRewriter, Settings, element, text};

    /// One `article.item`'s fields, as their text arrives.
    #[derive(Default)]
    struct Fields {
        id: String,
        name: String,
    }
    let read: RefCell<Vec<Fields>> = RefCell::new(Vec::new());
    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![
                element!("article.item", |_| {
                    read.borrow_mut().push(Fields::default());
                    Ok(())
                }),
                text!("article.item p.id", |chunk| {
                    if let Some(fields) = read.borrow_mut().last_mut() {
                        fields.id.push_str(chunk.as_str());
                    }
                    Ok(())
                }),
                text!("article.item h1.name", |chunk| {
                    if let Some(fields) = read.borrow_mut().last_mut() {
                        fields.name.push_str(chunk.as_str());
                    }
                    Ok(())
                }),
            ],
            ..Settings::new()
        },
        |_: &[u8]| {},
    );
    let parsing =
        |error: lol_html::errors::RewritingError| format!("parsing {}: {error}", page.get_url());
    rewriter.write(page.get_html_bytes_u8()).map_err(parsing)?;
    rewriter.end().map_err(parsing)?;
    let records: Vec<serde_json::Value> = read
        .into_inner()
        .into_iter()
        .map(|fields| (collapsed(&fields.id), collapsed(&fields.name)))
        .filter(|(id, name)| !id.is_empty() || !name.is_empty())
        .map(|(id, name)| serde_json::json!({ "id": id, "name": name }))
        .collect();
    Ok(serde_json::json!({
        "url": page.get_url(),
        "rendered": page.bytes_transferred.is_some(),
        "records": records,
    }))
}

#[cfg(not(feature = "smart"))]
fn records(_page: &spider::page::Page) -> Line {
    Err("the auto scenario takes the `smart` feature's executable".to_owned())
}

impl Fetched {
    /// Writes the pages still being read and flushes the file, once the
    /// stream is drained.
    async fn finish(mut self) -> Result<Self, String> {
        if let Some(written) = self.written.as_mut() {
            while let Some(read) = written.reading.join_next().await {
                written.write(read)?;
            }
            written.file.flush().map_err(|error| format!("writing {PAGES_FILE}: {error}"))?;
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

/// How Spider crawls a scenario's site.
enum Mode {
    /// Over HTTP alone (`crawl_raw()`).
    Http,
    /// Each page rendered in Chrome (`crawl()`).
    Chrome,
    /// Over HTTP, each page rendered in Chrome where Spider finds it needs
    /// one (`crawl_smart()`).
    Smart,
}

async fn crawl(arguments: &Arguments) -> Result<Fetched, String> {
    let mode = match arguments.scenario.as_str() {
        "raw" | "hostile" => Mode::Http,
        "extraction" if cfg!(feature = "extraction") => Mode::Http,
        "extraction" => {
            return Err("the extraction scenario takes the `extraction` feature's executable".to_owned());
        }
        "browser" if cfg!(feature = "chrome") => Mode::Chrome,
        "browser" => return Err("the browser scenario takes the `chrome` feature's executable".to_owned()),
        "auto" if cfg!(feature = "smart") => Mode::Smart,
        "auto" => return Err("the auto scenario takes the `smart` feature's executable".to_owned()),
        other => return Err(format!("Spider runs no `{other}` scenario")),
    };
    let mut site = Website::new(&arguments.url);
    // The hostile pages' pass at defaults (#2439) sets nothing: Spider's
    // own defaults, its own limit of requests in flight and robots.txt not
    // read.
    if arguments.scenario != "hostile" {
        site.with_concurrency_limit(Some(arguments.concurrency))
            .with_respect_robots_txt(false);
    }
    if let Some(host) = &arguments.host {
        let mut headers = HeaderMap::new();
        headers.insert(
            HOST,
            HeaderValue::from_str(host).map_err(|error| error.to_string())?,
        );
        site.with_headers(Some(headers));
    }
    let read: Option<fn(&spider::page::Page) -> Line> = match arguments.scenario.as_str() {
        "extraction" => Some(extracted),
        "auto" => Some(records),
        _ => None,
    };
    let written = match read {
        Some(read) => Some(Written {
            file: std::io::BufWriter::new(
                std::fs::File::create(PAGES_FILE).map_err(|error| format!("creating {PAGES_FILE}: {error}"))?,
            ),
            read,
            reading: tokio::task::JoinSet::new(),
        }),
        None => None,
    };
    // Every page is read from the stream, drained once the crawl ends:
    // the lab found the crawl's end may come before its last page is read.
    // The reader is a task of its own, as Spider's documentation subscribes
    // (`Website::subscribe`'s example): beside the crawl in one task, it
    // read only when the crawl's own future yielded, and under Callgrind a
    // channel of 64 or of 128 pages lost some (#2213).
    // A bare `tokio::spawn`: this executable stands outside the workspace
    // and has no tracker; the task is awaited below, so none outlives the
    // crawl.
    let mut pages = site.subscribe(arguments.capacity);
    let (done, mut ended) = tokio::sync::oneshot::channel::<()>();
    let reading = tokio::spawn(async move {
        let mut fetched = Fetched { written, ..Fetched::default() };
        loop {
            tokio::select! {
                page = pages.recv() => match page {
                    Ok(page) => fetched.add(page)?,
                    Err(RecvError::Lagged(lost)) => return Err(format!("the page stream lost {lost} pages")),
                    Err(RecvError::Closed) => return fetched.finish().await,
                },
                _ = &mut ended => loop {
                    match pages.try_recv() {
                        Ok(page) => fetched.add(page)?,
                        Err(TryRecvError::Lagged(lost)) => return Err(format!("the page stream lost {lost} pages")),
                        Err(_) => return fetched.finish().await,
                    }
                },
            }
        }
    });
    match mode {
        Mode::Http => site.crawl_raw().await,
        Mode::Chrome => site.crawl().await,
        Mode::Smart => site.crawl_smart().await,
    }
    // The receiver lives until the stream is drained.
    let _ = done.send(());
    reading
        .await
        .map_err(|error| format!("reading the page stream: {error}"))?
}

#[tokio::main]
async fn main() -> ExitCode {
    if admission::asked() {
        return match admission::run().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("goodfetch-compare-spider: {error}");
                ExitCode::FAILURE
            }
        };
    }
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
