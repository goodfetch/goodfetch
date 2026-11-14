//! One crawl with crawlberg, for `just compare` (#2218): its engine's
//! streaming crawl (`create_engine`, then `CrawlEngineHandle::crawl_stream`,
//! which hands each page on as it is crawled rather than holding the whole
//! crawl's pages until it ends) from `--url`, every page of the site,
//! `--concurrency` requests in flight. Every crawlberg crawl is held to its
//! seed's host, renders each page to Markdown (its `standard` preset, its
//! default) and refuses private addresses unless told otherwise, so every
//! scenario allows them (`allow_private_networks`): the Proving ground
//! serves on the loopback address. With `--host`, when given, that host is
//! sent as each request's `Host` (`custom_headers`, which crawlberg sends
//! to the seed's host only, here the Proving ground's address).
//! `--scenario` names the matrix's scenario:
//!
//! - `raw`: robots.txt not obeyed (`respect_robots_txt`, off by default),
//!   no delay between requests to the site (`rate_limit_ms` 0 where its
//!   default is 200 ms), `--concurrency` requests in flight
//!   (`max_concurrent`), no page or depth bound (its defaults);
//! - `extraction`: `raw` with its main content alone
//!   (`content.preprocessing_preset` `aggressive`, the setting its own
//!   API's `onlyMainContent` turns on), each page's URL, title
//!   (`metadata.title`) and Markdown content (`markdown.content`) written
//!   to `./pages.jsonl` as the page comes;
//! - `browser`: `raw` with each page rendered in `--browser`
//!   (`browser.mode: always`, `browser.chrome_path`), its Chrome backend;
//! - `hostile`: the hostile pages' pass at each tool's defaults (#2439):
//!   its defaults but for the private addresses and the `Host` above:
//!   robots.txt not obeyed, 200 ms between requests to a host, 10
//!   requests in flight; `--concurrency` is unread.
//!
//! crawlberg has no durable cell: its frontier lives in memory and no crawl
//! resumes; nor a polite one: it follows no page link off its seed's host,
//! and the multi-domain site spans 1,000 domains (the matrix says why).
//!
//! The crawl-like scenario (#2284) runs `raw` on an `https` site, whose
//! certificate authority `SSL_CERT_FILE` names, which crawlberg's TLS
//! (rustls, with the platform's verifier) reads as the system's roots.
//!
//! The working folder is the trial's home. It prints one line, `pages=<n>`,
//! the pages crawlberg handed on; the comparison's same-work check reads
//! the Proving ground's own observations instead, and refuses a trial that
//! missed a page.
//!
//!   goodfetch-compare-crawlberg --url http://127.0.0.1:<port>/sites/1000/1 \
//!     --host scale.test:<port> --concurrency 16 --scenario raw
#![expect(
    clippy::disallowed_methods,
    reason = "a comparison side, no Product crate: its `main` builds its runtime, and its extraction writes `pages.jsonl` through std's file"
)]

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use crawlberg::{BrowserMode, CrawlConfig, CrawlEvent, CrawlPageResult, CrawlStreamRequest};
use futures::StreamExt as _;

struct Arguments {
    url: String,
    host: Option<String>,
    concurrency: usize,
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
    let usage = "usage: goodfetch-compare-crawlberg --url <url> [--host <host:port>] --concurrency <n> --scenario <raw|extraction|browser|hostile> [--browser <chrome>]";
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
                    other => return Err(format!("crawlberg runs no `{other}` scenario")),
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

fn configuration(arguments: &Arguments) -> CrawlConfig {
    let mut config = CrawlConfig::builder().allow_private_networks(true).build();
    if let Some(host) = &arguments.host {
        config
            .custom_headers
            .insert("Host".to_owned(), host.clone());
    }
    if arguments.scenario == Scenario::Hostile {
        return config;
    }
    config.respect_robots_txt = false;
    config.rate_limit_ms = Some(0);
    config.max_concurrent = Some(arguments.concurrency);
    if arguments.scenario == Scenario::Extraction {
        "aggressive".clone_into(&mut config.content.preprocessing_preset);
    }
    if arguments.scenario == Scenario::Browser {
        config.browser.mode = BrowserMode::Always;
        config.browser.chrome_path = arguments.browser.as_ref().map(PathBuf::from);
    }
    config
}

/// One page as a line of `./pages.jsonl`: its URL, title and Markdown
/// content.
fn write_page(lines: &mut impl Write, page: &CrawlPageResult) -> io::Result<()> {
    let line = serde_json::json!({
        "url": page.url,
        "title": page.metadata.title,
        "text": page.markdown.as_ref().map(|markdown| markdown.content.as_str()),
    });
    writeln!(lines, "{line}")
}

async fn crawl(arguments: Arguments) -> Result<u64, String> {
    let engine = crawlberg::create_engine(Some(configuration(&arguments)))
        .map_err(|error| error.to_string())?;
    let mut lines = match arguments.scenario {
        Scenario::Extraction => Some(io::BufWriter::new(
            std::fs::File::create("pages.jsonl").map_err(|error| error.to_string())?,
        )),
        Scenario::Raw | Scenario::Browser | Scenario::Hostile => None,
    };
    let crawled = async {
        let mut events = engine
            .crawl_stream(CrawlStreamRequest {
                url: arguments.url.clone(),
            })
            .await
            .map_err(|error| error.to_string())?;
        let mut pages = 0_u64;
        let mut failure = None;
        while let Some(event) = events.next().await {
            match event.map_err(|error| error.to_string())? {
                CrawlEvent::Page { result } => {
                    pages += 1;
                    if let Some(lines) = &mut lines {
                        write_page(lines, &result).map_err(|error| error.to_string())?;
                    }
                }
                // A page that failed is the same-work check's to count; a
                // crawl that crawled nothing failed.
                CrawlEvent::Error { url, error } => {
                    failure.get_or_insert(format!("{url}: {error}"));
                }
                CrawlEvent::Complete { .. } => break,
            }
        }
        if let Some(lines) = &mut lines {
            lines.flush().map_err(|error| error.to_string())?;
        }
        match failure {
            Some(failure) if pages == 0 => Err(failure),
            _ => Ok(pages),
        }
    }
    .await;
    crawlberg::shutdown_engine(engine).await;
    crawled
}

#[tokio::main]
async fn main() -> ExitCode {
    let crawled = match arguments() {
        Ok(arguments) => crawl(arguments).await,
        Err(error) => Err(error),
    };
    match crawled {
        Ok(pages) => {
            println!("pages={pages}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("goodfetch-compare-crawlberg: {error}");
            ExitCode::FAILURE
        }
    }
}
