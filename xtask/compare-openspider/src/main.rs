//! One crawl with OpenSpider, for `just compare` (#2216): its engine's
//! spider crawl (`os_engine::start_crawl`, the call its own command line
//! makes) from `--url`, `--concurrency` threads, every page of the site, on
//! its own HTTP fetcher. With `--host`, when given, the crawl starts on
//! that host's name, which its resolver (`network.resolve`, as `curl
//! --resolve` would) answers with `--url`'s address, so the Proving ground
//! serves its family on the loopback address. Every OpenSpider crawl keeps
//! its crawl file, an SQLite database, here `./crawl.ospider`, on its
//! `Balanced` storage profile, its command line's. The crawl analysis its
//! command line runs once a crawl ends (its SEO reports) is not run.
//! `--scenario` names the matrix's scenario:
//!
//! - `raw`: robots.txt not obeyed (`robots.mode: ignore`) and the rest at
//!   its defaults: at most 6 connections to a host
//!   (`connections_per_host: auto`), each carrying one request at a time
//!   under HTTP/1.1, so at most 6 requests in flight on a host whatever
//!   the threads, no rate limit, its adaptive throttle on, images, style sheets, scripts and canonicals followed on the
//!   host (none on the Proving ground's pages);
//! - `extraction`: `raw` with one custom extractor, the text of each page's
//!   `main` (`custom_extraction`, CSS, text output); once the crawl ends,
//!   each crawled page's URL, title and that text are read from the crawl
//!   file and written to `./pages.jsonl`;
//! - `browser`: `raw` with each page rendered in `--browser`
//!   (`rendering.mode: javascript`, `rendering.chromium_path`), its tabs at
//!   their default, as many as threads up to the CPU's cores;
//! - `hostile`: the hostile pages' pass at each tool's defaults (#2439):
//!   `CrawlConfig::default()` with nothing set but the resolver above:
//!   robots.txt obeyed and its sitemaps read, 5 threads, its adaptive
//!   throttle on; `--concurrency` is unread.
//!
//! OpenSpider has no durable cell: its crawl file holds nothing yet when
//! the comparison kills it halfway, and its resume of that file fetches
//! nothing; no polite cell: its only pace setting holds the whole crawl to
//! its rate; nor cells of 64 or 256 requests in flight, beyond its 6 on a
//! host (the matrix says why).
//!
//! The crawl-like scenario (#2284) runs `raw` on an `https` site, whose
//! certificate authority `SSL_CERT_FILE` names, which its fetcher also
//! trusts as a certificate a user adds (`network.trusted_certificates`).
//!
//! The working folder is the trial's home. It prints one line, `pages=<n>`,
//! the pages its command line counts as crawled, from the crawl file; the
//! comparison's same-work check reads the Proving ground's own observations
//! instead, and refuses a trial that missed a page.
//!
//!   goodfetch-compare-openspider --url http://127.0.0.1:<port>/sites/1000/1 \
//!     --host scale.test:<port> --concurrency 16 --scenario raw
#![expect(
    clippy::disallowed_methods,
    reason = "a comparison side, no Product crate: its `main` builds its runtime and reads `SSL_CERT_FILE`, and once its crawl has ended it writes `pages.jsonl` through std's file"
)]

use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use os_engine::{CrawlRequest, CrawlSource, EngineDeps, start_crawl};
use os_fetch::{FetchConfig, HttpFetcher, MemorySecretStore};
use os_model::{
    CrawlConfig, Extractor, ExtractorMode, ExtractorOutput, RandomIds, RenderingMode, RobotsMode,
    SystemClock,
};
use os_store::{CrawlFile, OpenMode, StorageProfile};

struct Arguments {
    url: String,
    host: Option<String>,
    concurrency: u32,
    scenario: Scenario,
    browser: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Raw,
    Extraction,
    Browser,
    Hostile,
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: goodfetch-compare-openspider --url <url> [--host <host:port>] --concurrency <n> --scenario <raw|extraction|browser|hostile> [--browser <chrome>]";
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
            "--browser" => browser = Some(value),
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
                    "extraction" => Scenario::Extraction,
                    "browser" => Scenario::Browser,
                    "hostile" => Scenario::Hostile,
                    other => return Err(format!("OpenSpider runs no `{other}` scenario")),
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

/// The crawl file, in the trial's home.
fn crawl_file() -> PathBuf {
    PathBuf::from(format!("crawl.{}", os_store::registry::EXTENSION))
}

/// The page the crawl starts on, and the resolver rule that answers its
/// host: `--url` itself without `--host`; with it, `--url` on that host's
/// name, which the rule answers with `--url`'s address.
fn start(url: &str, host: Option<&str>) -> Result<(String, Option<String>), String> {
    let Some(host) = host else {
        return Ok((url.to_owned(), None));
    };
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("--url {url} has no scheme"))?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let address = authority
        .rsplit_once(':')
        .map_or(authority, |(address, _)| address);
    let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
    Ok((
        format!("{scheme}://{host}{path}"),
        Some(format!("{name}={address}")),
    ))
}

fn configuration(arguments: &Arguments, rule: Option<String>) -> CrawlConfig {
    let mut config = CrawlConfig::default();
    config.network.resolve.extend(rule);
    if arguments.scenario == Scenario::Hostile {
        return config;
    }
    config.speed.threads = arguments.concurrency;
    config.robots.mode = RobotsMode::Ignore;
    match arguments.scenario {
        Scenario::Extraction => config.custom_extraction.extractors.push(Extractor {
            name: "main".to_owned(),
            mode: ExtractorMode::Css,
            expression: "main".to_owned(),
            attribute: None,
            output: ExtractorOutput::Text,
        }),
        Scenario::Browser => {
            config.rendering.mode = RenderingMode::JavaScript;
            config
                .rendering
                .chromium_path
                .clone_from(&arguments.browser);
        }
        Scenario::Raw | Scenario::Hostile => {}
    }
    config
}

fn dependencies() -> EngineDeps {
    EngineDeps {
        clock: Arc::new(SystemClock),
        ids: Arc::new(RandomIds),
        profile: StorageProfile::Balanced,
        secrets: Arc::new(MemorySecretStore::new()),
        sink_factory: None,
        on_finished: None,
        owner_kind: "cli",
        renderer: None,
    }
}

/// OpenSpider's fetcher for `config`, trusting `authority`, the
/// certificate authority `SSL_CERT_FILE` names, as a certificate a user
/// adds.
fn fetcher(config: &CrawlConfig, authority: Option<&str>) -> Result<Arc<HttpFetcher>, String> {
    let secrets = MemorySecretStore::new();
    let mut fetch = FetchConfig::from_crawl_config(config, &secrets, Arc::new(SystemClock))
        .map_err(|error| error.to_string())?;
    if let Some(authority) = authority {
        fetch
            .network
            .trusted_certificates
            .push(authority.to_owned());
    }
    HttpFetcher::new(fetch)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

async fn crawl(arguments: Arguments, authority: Option<String>) -> Result<u64, String> {
    let (start_url, rule) = start(&arguments.url, arguments.host.as_deref())?;
    let config = configuration(&arguments, rule);
    let file = crawl_file();
    let request = CrawlRequest {
        config: config.clone(),
        source: CrawlSource::Spider { start_url },
        file: file.clone(),
        project: "compare".to_owned(),
        name: "compare".to_owned(),
    };
    let handle = start_crawl(
        fetcher(&config, authority.as_deref())?,
        request,
        dependencies(),
    )
    .await
    .map_err(|error| error.to_string())?;
    handle.wait().await.map_err(|error| error.to_string())?;
    let crawl = CrawlFile::open(&file, OpenMode::ReadOnly).map_err(|error| error.to_string())?;
    if arguments.scenario == Scenario::Extraction {
        write_pages(&crawl)?;
    }
    // Its command line's count of the crawled pages.
    let crawled: i64 = crawl
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM urls WHERE state = 'done' AND is_internal = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    u64::try_from(crawled).map_err(|error| error.to_string())
}

/// Each crawled page's URL, title and its extractor's first value, as a
/// line of `./pages.jsonl`.
fn write_pages(crawl: &CrawlFile) -> Result<(), String> {
    let mut statement = crawl
        .conn()
        .prepare(
            "SELECT url, title_1, (SELECT value FROM custom_extraction_values \
             WHERE url_id = urls.id AND def_index = 0 ORDER BY number LIMIT 1) \
             FROM urls WHERE state = 'done' AND is_internal = 1",
        )
        .map_err(|error| error.to_string())?;
    let pages = statement
        .query_map([], |row| {
            Ok(serde_json::json!({
                "url": row.get::<_, String>(0)?,
                "title": row.get::<_, Option<String>>(1)?,
                "text": row.get::<_, Option<String>>(2)?,
            }))
        })
        .map_err(|error| error.to_string())?;
    let mut lines = io::BufWriter::new(
        std::fs::File::create("pages.jsonl").map_err(|error| error.to_string())?,
    );
    for page in pages {
        let page = page.map_err(|error| error.to_string())?;
        writeln!(lines, "{page}").map_err(|error| error.to_string())?;
    }
    lines.flush().map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let authority = std::env::var_os("SSL_CERT_FILE")
        .map(|authority| {
            authority
                .into_string()
                .map_err(|_| "SSL_CERT_FILE is no text".to_owned())
        })
        .transpose();
    let crawled = match (arguments(), authority) {
        (Ok(arguments), Ok(authority)) => crawl(arguments, authority).await,
        (Err(error), _) | (_, Err(error)) => Err(error),
    };
    match crawled {
        Ok(pages) => {
            println!("pages={pages}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("goodfetch-compare-openspider: {error}");
            ExitCode::FAILURE
        }
    }
}
