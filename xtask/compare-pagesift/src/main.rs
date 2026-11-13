//! One crawl with pagesift, for `just compare` (#2215): its `Scraper`'s
//! breadth-first crawl from `--url`, at most `--concurrency` requests at
//! once, every page of the site, as its own crawl benchmark drives it
//! (`crates/pagesift/examples/crawl_bench.rs`), with its own allocator
//! (`SmallCache`, tuned at start) as its executable has. pagesift sends no
//! caller's `Host` header (it refuses one), so with `--host`, when given,
//! the crawl starts on that host's name, which a resolver of its own
//! (`FetcherConfig::dns`) answers with `--url`'s address, as `curl
//! --resolve` would, and the Proving ground serves its family on the
//! loopback address. `--scenario` names the matrix's scenario:
//!
//! - `raw`: nothing kept on disk, robots.txt not obeyed, no per-site pacing
//!   (`politeness` off, its administrator's switch), each page's links its
//!   only format, links followed on the same host (`crawlEntireDomain`);
//! - `durable`: `raw` run as a job of its HTTP API, in process, whose store
//!   under `./store` keeps the job, its frontier and its pages: the process
//!   the comparison starts again after a kill opens the store, which resumes
//!   the job where the killed one left it, and waits for it to end;
//! - `polite`: robots.txt obeyed and one request start a second on each
//!   site (`politeness` on, its default, at a 1 s interval), every link
//!   followed (`allowExternalLinks`), since the multi-domain site spans many
//!   hosts and links nothing else;
//! - `extraction`: `raw` with each page's main content as Markdown (its
//!   `onlyMainContent`, the default), its URL, title and Markdown written
//!   to `./pages.jsonl`;
//! - `browser`: `raw` with each page rendered in `--browser`, its browser
//!   engine.
//!
//! The working folder is the trial's home. It prints one line, `pages=<n>`,
//! the pages pagesift crawled (after a resume, the killed process's too),
//! and its failed pages' errors on stderr; the comparison's same-work check
//! reads the Proving ground's own observations instead, and refuses a trial
//! that missed a page.
//!
//!   goodfetch-compare-pagesift --url http://127.0.0.1:<port>/sites/1000/1 \
//!     --host scale.test:<port> --concurrency 16 --scenario raw
#![expect(
    clippy::disallowed_methods,
    reason = "a comparison side, no Product crate: its `main` builds its runtime"
)]

use std::io::{self, Write as _};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use pagesift::api::types::{CrawlRequest, Document};
use pagesift::api::{ApiError, BoxFuture, CrawlSink, Engine};
use pagesift::{Scraper, ScraperConfig};
use pagesift_net::{DnsLookup, SystemLookup, Url};
use tower::ServiceExt as _;

#[global_allocator]
static ALLOCATOR: pagesift_sys::SmallCache = pagesift_sys::SmallCache;

/// More pages than any site of the comparison, at the API's own maximum
/// (`MAX_CRAWL_LIMIT`), so the site, not the limit, ends a crawl.
const LIMIT: usize = 100_000;

/// The file in the trial's home that names the durable scenario's job.
const JOB: &str = "job";

/// How often the durable scenario asks its job whether it has ended.
const POLL: Duration = Duration::from_millis(50);

struct Arguments {
    url: String,
    host: Option<String>,
    concurrency: usize,
    scenario: Scenario,
    browser: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Raw,
    Durable,
    Polite,
    Extraction,
    Browser,
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: goodfetch-compare-pagesift --url <url> [--host <host:port>] --concurrency <n> --scenario <raw|durable|polite|extraction|browser> [--browser <executable>]";
    let mut url = None;
    let mut host = None;
    let mut concurrency = None;
    let mut scenario = None;
    let mut browser = None;
    let mut given = std::env::args().skip(1);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(usage)?;
        match name.as_str() {
            "--url" => url = Some(value),
            "--host" => host = Some(value),
            "--browser" => browser = Some(PathBuf::from(value)),
            "--concurrency" => {
                concurrency = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|concurrency| *concurrency > 0)
                        .ok_or("--concurrency takes a count of at least 1")?,
                );
            }
            "--scenario" => {
                scenario = Some(match value.as_str() {
                    "raw" => Scenario::Raw,
                    "durable" => Scenario::Durable,
                    "polite" => Scenario::Polite,
                    "extraction" => Scenario::Extraction,
                    "browser" => Scenario::Browser,
                    other => return Err(format!("pagesift runs no `{other}` scenario")),
                });
            }
            _ => return Err(usage.to_owned()),
        }
    }
    let scenario = scenario.ok_or(usage)?;
    if scenario == Scenario::Browser && browser.is_none() {
        return Err("the browser scenario takes --browser".to_owned());
    }
    Ok(Arguments {
        url: url.ok_or(usage)?,
        host,
        concurrency: concurrency.ok_or(usage)?,
        scenario,
        browser,
    })
}

/// Answers one host name with one address, every other name with the
/// system's resolver.
struct Pinned {
    name: String,
    address: IpAddr,
}

impl DnsLookup for Pinned {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> pagesift_net::BoxFuture<'a, io::Result<Vec<IpAddr>>> {
        if host == self.name {
            Box::pin(async move { Ok(vec![self.address]) })
        } else {
            SystemLookup.lookup(host)
        }
    }
}

/// The crawl's first page and pagesift's settings for `arguments`.
fn settings(arguments: &Arguments) -> Result<(String, ScraperConfig), String> {
    let mut config = ScraperConfig::default();
    // The Proving ground is on the loopback address.
    config.fetcher.ssrf.allow_private_network = true;
    let mut start = Url::parse(&arguments.url).map_err(|error| error.to_string())?;
    if let Some(host) = &arguments.host {
        let address = start
            .host_str()
            .and_then(|address| address.parse().ok())
            .ok_or("with --host, --url is on an IP address")?;
        let name = host.split(':').next().unwrap_or(host).to_owned();
        start
            .set_host(Some(&name))
            .map_err(|error| error.to_string())?;
        config.fetcher.dns = Arc::new(Pinned { name, address });
    }
    let rate = &mut config.policy.rate;
    // At most `--concurrency` in flight, as the other tools: its starting
    // value and its hard ceiling, which its controller stays under.
    rate.global_concurrency = arguments.concurrency;
    rate.global_ceiling = Some(arguments.concurrency);
    if arguments.scenario == Scenario::Polite {
        rate.default_interval = Duration::from_secs(1);
    } else {
        rate.politeness = false;
        config.policy.respect_robots = false;
    }
    if let Some(browser) = &arguments.browser {
        config.browser.source = pagesift_cdp::BrowserSource::Executable(browser.clone());
    }
    Ok((start.into(), config))
}

/// The crawl request for `arguments`' scenario from `start`, as pagesift's
/// API takes it.
fn request(arguments: &Arguments, start: &str) -> serde_json::Value {
    let polite = arguments.scenario == Scenario::Polite;
    let (formats, engine) = match arguments.scenario {
        Scenario::Raw | Scenario::Durable | Scenario::Polite => (vec!["links"], "http"),
        Scenario::Extraction => (vec!["markdown"], "http"),
        Scenario::Browser => (vec!["links"], "browser"),
    };
    serde_json::json!({
        "url": start,
        "limit": LIMIT,
        "sitemap": "skip",
        "crawlEntireDomain": true,
        "allowExternalLinks": polite,
        "ignoreRobotsTxt": !polite,
        "maxConcurrency": arguments.concurrency,
        "scrapeOptions": { "formats": formats, "engine": engine },
    })
}

/// Counts the crawl's pages, prints its failures, and in the extraction
/// scenario writes each page's URL, title and Markdown.
struct Count {
    pages: AtomicU64,
    output: Option<Mutex<io::BufWriter<std::fs::File>>>,
}

impl CrawlSink for Count {
    fn page(&self, document: Document) -> BoxFuture<'_, ()> {
        self.pages.fetch_add(1, Ordering::Relaxed);
        if let Some(output) = &self.output {
            let metadata = &document.metadata;
            let line = serde_json::json!({
                "url": metadata.url.as_ref().or(metadata.source_url.as_ref()),
                "title": metadata.title,
                "text": document.markdown,
            });
            let mut output = output.lock().expect("invariant: no writer panics");
            if let Err(error) = writeln!(output, "{line}") {
                eprintln!("goodfetch-compare-pagesift: writing pages.jsonl: {error}");
            }
        }
        Box::pin(async {})
    }

    fn error<'a>(&'a self, url: &'a str, error: ApiError) -> BoxFuture<'a, ()> {
        eprintln!(
            "goodfetch-compare-pagesift: {url}: {} {}",
            error.code.as_str(),
            error.message
        );
        Box::pin(async {})
    }

    fn total(&self, _: usize) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn cancelled(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }
}

async fn crawl(arguments: Arguments) -> Result<u64, String> {
    let (start, config) = settings(&arguments)?;
    let request = request(&arguments, &start);
    let scraper = Scraper::new(config).map_err(|error| error.to_string())?;
    if arguments.scenario == Scenario::Durable {
        return resume(scraper, request).await;
    }
    let request: CrawlRequest =
        serde_json::from_value(request).map_err(|error| error.to_string())?;
    let output = if arguments.scenario == Scenario::Extraction {
        let file = std::fs::File::create("pages.jsonl").map_err(|error| error.to_string())?;
        Some(Mutex::new(io::BufWriter::new(file)))
    } else {
        None
    };
    let count = Arc::new(Count {
        pages: AtomicU64::new(0),
        output,
    });
    scraper
        .crawl(&request, count.clone())
        .await
        .map_err(|error| format!("{} {}", error.code.as_str(), error.message))?;
    if let Some(output) = &count.output {
        output
            .lock()
            .expect("invariant: no writer panics")
            .flush()
            .map_err(|error| error.to_string())?;
    }
    Ok(count.pages.load(Ordering::Relaxed))
}

/// The durable scenario: the crawl as a job of pagesift's API, kept in its
/// store under `./store`. The first process starts the job; one started
/// again after a kill finds it in the store, which resumes it, and both
/// wait for it to end.
async fn resume(scraper: Scraper, request: serde_json::Value) -> Result<u64, String> {
    let folder = Path::new("store");
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let store = pagesift::store::Store::open(pagesift::store::Config::new(folder))
        .map_err(|error| format!("opening the store: {error}"))?;
    let engine: Arc<dyn Engine> = Arc::new(scraper);
    let api = pagesift::api::router(
        engine,
        pagesift::api::Config {
            store: Some(store),
            ..pagesift::api::Config::default()
        },
    );
    let id = match job(&api).await? {
        Some(id) => id,
        None => {
            let started = call(&api, Method::POST, "/v1/crawl", Some(request)).await?;
            let id = started["id"]
                .as_str()
                .ok_or_else(|| format!("the crawl started with no id: {started}"))?
                .to_owned();
            // Written whole or not at all, as a kill may come at any time.
            std::fs::write("job.new", &id).map_err(|error| error.to_string())?;
            std::fs::rename("job.new", JOB).map_err(|error| error.to_string())?;
            id
        }
    };
    // Past every page, so the answer carries none.
    let status = format!("/v1/crawl/{id}?skip={}", usize::MAX);
    loop {
        let job = call(&api, Method::GET, &status, None).await?;
        match job["status"].as_str() {
            Some("scraping") => tokio::time::sleep(POLL).await,
            Some("completed") => {
                return job["completed"]
                    .as_u64()
                    .ok_or_else(|| format!("the job counts no pages: {job}"));
            }
            _ => return Err(format!("the job ended without completing: {job}")),
        }
    }
}

/// The job a killed process started: the one its file names, else the one
/// the store resumed before the file was written; none in a fresh home.
async fn job(api: &axum::Router) -> Result<Option<String>, String> {
    match std::fs::read_to_string(JOB) {
        Ok(id) => return Ok(Some(id)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let active = call(api, Method::GET, "/v1/crawl/active", None).await?;
    Ok(active["crawls"]
        .as_array()
        .and_then(|crawls| crawls.first())
        .and_then(|crawl| crawl["id"].as_str())
        .map(str::to_owned))
}

/// One call to the API, in process, and its JSON answer.
async fn call(
    api: &axum::Router,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .map_err(|error| error.to_string())?;
    let answer = api
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| error.to_string())?;
    let status = answer.status();
    let bytes = axum::body::to_bytes(answer.into_body(), usize::MAX)
        .await
        .map_err(|error| error.to_string())?;
    if status != StatusCode::OK {
        return Err(format!(
            "{path} answered {status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    pagesift_sys::tune_allocator();
    let crawled = match arguments() {
        Ok(arguments) => crawl(arguments).await,
        Err(usage) => Err(usage),
    };
    match crawled {
        Ok(pages) => {
            println!("pages={pages}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("goodfetch-compare-pagesift: {error}");
            ExitCode::FAILURE
        }
    }
}
