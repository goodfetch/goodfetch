//! One crawl with crawlee-rs, for `just compare` (#2214): its `HtmlCrawler`
//! from `--url`, at most `--concurrency` requests at once, every page of
//! the site, with `--host`, when given, sent as each request's `Host` so
//! the Proving ground serves its family on the loopback address.
//! `--scenario` names the matrix's scenario:
//!
//! - `raw`: nothing kept on disk (`persist_storage: false`), robots.txt not
//!   obeyed (its default), links followed on the same hostname
//!   (`enqueue_links`' default);
//! - `durable`: its storage kept on disk under `./storage` (its default),
//!   and not purged when a crawler starts, so the process the comparison
//!   starts again after a kill resumes the request queue the killed one
//!   left;
//! - `polite`: `raw` with robots.txt obeyed and one request start a second
//!   on each domain (`same_domain_delay`), every link followed, since the
//!   multi-domain site spans many hosts and links nothing else;
//! - `extraction`: `raw` with each page's URL, title and main text (the
//!   text of its `main`) given to `push_data`, its default Dataset kept on
//!   disk under `./storage`;
//! - `hostile`: the hostile pages' pass at each tool's defaults (#2439):
//!   `Configuration::default()` and the crawler's builder with no option but
//!   the `Host` it sends, so `max_concurrency` 200 (its default), robots.txt
//!   not obeyed and links on the same hostname; `--concurrency` is unread;
//! - `handled` (#2317): `raw` with each page's twenty fields read from its
//!   `Document`, each the first element its selector matches, as its text
//!   or one attribute, `null` when none matches, given with the page's URL
//!   to `push_data`, its default Dataset kept on disk under `./storage`.
//!
//! The crawl-like scenario (#2284) runs `raw` on an `https` site, which
//! its `reqwest` client fetches over HTTP/2 when ALPN offers it.
//!
//! The working folder is the trial's home. It prints one line, `pages=<n>`,
//! the requests crawlee-rs's statistics count as handled (after a resume,
//! the killed process's too), and its failed requests' errors on stderr;
//! the comparison's same-work check reads the Proving ground's own
//! observations instead, and refuses a trial that missed a page.
//!
//!   goodfetch-compare-crawlee-rs --url http://127.0.0.1:<port>/sites/1000/1 \
//!     --host scale.test:<port> --concurrency 16 --scenario raw
#![expect(
    clippy::disallowed_methods,
    reason = "a comparison side, no Product crate: its `main` builds its runtime"
)]

use std::process::ExitCode;
use std::time::Duration;

use crawlee::{
    Configuration, Document, EnqueueLinksOptions, EnqueueStrategy, HtmlContext, HtmlCrawler,
    HttpCrawlerOptions, Services,
};

struct Arguments {
    url: String,
    host: Option<String>,
    concurrency: usize,
    scenario: Scenario,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Raw,
    Durable,
    Polite,
    Extraction,
    Hostile,
    Handled,
}

/// The handled crawl's twenty fields of a page of the article site: each
/// one's name, selector and, for a link, the attribute it reads in place of
/// the text.
const FIELDS: [(&str, &str, Option<&str>); 20] = [
    ("title", "title", None),
    ("name", "article.item > h1.name", None),
    ("id", "article.item", Some("data-id")),
    ("home", "nav > a", Some("href")),
    ("home_label", "nav > a", None),
    ("header", "header > p", None),
    ("nav", "nav > p", None),
    ("footer", "footer > p", None),
    (
        "first_child",
        "ul.pages > li:nth-child(1) > a",
        Some("href"),
    ),
    ("first_child_name", "ul.pages > li:nth-child(1) > a", None),
    (
        "second_child",
        "ul.pages > li:nth-child(2) > a",
        Some("href"),
    ),
    ("second_child_name", "ul.pages > li:nth-child(2) > a", None),
    ("paragraph_1", "article.item > p:nth-of-type(1)", None),
    ("paragraph_2", "article.item > p:nth-of-type(2)", None),
    ("paragraph_3", "article.item > p:nth-of-type(3)", None),
    ("paragraph_4", "article.item > p:nth-of-type(4)", None),
    ("paragraph_5", "article.item > p:nth-of-type(5)", None),
    ("paragraph_6", "article.item > p:nth-of-type(6)", None),
    ("paragraph_7", "article.item > p:nth-of-type(7)", None),
    ("paragraph_8", "article.item > p:nth-of-type(8)", None),
];

/// The handled crawl's fields of `document` with its `url`, `null` where
/// none matches.
fn fields(url: &str, document: &Document) -> Result<serde_json::Value, String> {
    let mut read = serde_json::Map::with_capacity(FIELDS.len() + 1);
    read.insert("url".to_owned(), url.into());
    for (name, selector, attribute) in FIELDS {
        let element = document
            .select_first(selector)
            .map_err(|error| error.to_string())?;
        let value = match (element, attribute) {
            (None, _) => serde_json::Value::Null,
            (Some(element), None) => element.text().into(),
            (Some(element), Some(attribute)) => element.attr(attribute).into(),
        };
        read.insert(name.to_owned(), value);
    }
    Ok(read.into())
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: goodfetch-compare-crawlee-rs --url <url> [--host <host:port>] --concurrency <n> --scenario <raw|durable|polite|extraction|hostile|handled>";
    let mut url = None;
    let mut host = None;
    let mut concurrency = None;
    let mut scenario = None;
    let mut given = std::env::args().skip(1);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(usage)?;
        match name.as_str() {
            "--url" => url = Some(value),
            "--host" => host = Some(value),
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
                    "hostile" => Scenario::Hostile,
                    "handled" => Scenario::Handled,
                    other => return Err(format!("crawlee-rs runs no `{other}` scenario")),
                });
            }
            _ => return Err(usage.to_owned()),
        }
    }
    Ok(Arguments {
        url: url.ok_or(usage)?,
        host,
        concurrency: concurrency.ok_or(usage)?,
        scenario: scenario.ok_or(usage)?,
    })
}

async fn crawl(arguments: Arguments) -> Result<u64, String> {
    let scenario = arguments.scenario;
    let configuration = if scenario == Scenario::Hostile {
        Configuration::default()
    } else {
        Configuration {
            persist_storage: matches!(
                scenario,
                Scenario::Durable | Scenario::Extraction | Scenario::Handled
            ),
            // The resumed process keeps what the killed one left; the first
            // starts in an empty home.
            purge_on_start: scenario != Scenario::Durable,
            ..Configuration::default()
        }
    };
    // crawlee-rs accepts any certificate by default: outside the hostile
    // pass, an `https` site's is verified through the platform's
    // verifier, which on Linux reads `SSL_CERT_FILE` in place of the
    // system's roots (macOS's ignores it).
    let mut options = HttpCrawlerOptions::default();
    if scenario != Scenario::Hostile {
        options.ignore_tls_errors = false;
    }
    if let Some(host) = arguments.host {
        options = options.pre_navigation_hook(move |mut context| {
            let host = host.clone();
            async move {
                context
                    .request_mut()
                    .headers
                    .insert("Host".to_owned(), host);
                Ok(context)
            }
        });
    }
    let strategy = if scenario == Scenario::Polite {
        EnqueueStrategy::All
    } else {
        EnqueueStrategy::SameHostname
    };
    // At most `--concurrency` in flight, as the other tools. Its autoscaled
    // pool starts at the crawler's desired concurrency and grows toward the
    // limit as the system allows: crawlee-rs's own behaviour, kept, as
    // Crawlee's side keeps Crawlee's.
    let mut builder = HtmlCrawler::builder_with_options(options)
        .services(Services::from_configuration(configuration));
    if scenario != Scenario::Hostile {
        builder = builder.max_concurrency(arguments.concurrency);
    }
    if scenario == Scenario::Polite {
        builder = builder
            .respect_robots_txt(true)
            .same_domain_delay(Duration::from_secs(1));
    }
    let crawler = builder
        .request_handler(move |context: HtmlContext| async move {
            if scenario == Scenario::Extraction {
                let (title, text) = context
                    .with_html(|document| -> Result<_, String> {
                        let text = document
                            .select("main")
                            .map_err(|error| error.to_string())?
                            .text();
                        Ok((document.title(), text.trim().to_owned()))
                    })
                    .await?
                    .map_err(anyhow::Error::msg)?;
                context.push_data(&serde_json::json!({
                    "url": context.url().as_str(),
                    "title": title,
                    "text": text,
                }))?;
            } else if scenario == Scenario::Handled {
                let url = context.url().as_str().to_owned();
                let read = context
                    .with_html(move |document| fields(&url, document))
                    .await?
                    .map_err(anyhow::Error::msg)?;
                context.push_data(&read)?;
            }
            context
                .enqueue_links(EnqueueLinksOptions::new().strategy(strategy))
                .await?;
            Ok(())
        })
        .build()
        .map_err(|error| error.to_string())?;
    let statistics = crawler
        .run([arguments.url])
        .await
        .map_err(|error| error.to_string())?;
    // A failed request is no failed run: the same-work check names the
    // trial that missed a page, as for the other tools.
    if statistics.requests_failed > 0 {
        eprintln!(
            "goodfetch-compare-crawlee-rs: {} requests failed: {:?}",
            statistics.requests_failed, statistics.errors
        );
    }
    Ok(statistics.requests_succeeded)
}

#[tokio::main]
async fn main() -> ExitCode {
    let handled = match arguments() {
        Ok(arguments) => crawl(arguments).await,
        Err(usage) => Err(usage),
    };
    match handled {
        Ok(pages) => {
            println!("pages={pages}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("goodfetch-compare-crawlee-rs: {error}");
            ExitCode::FAILURE
        }
    }
}
