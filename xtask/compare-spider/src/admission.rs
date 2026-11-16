//! The admission scenario, Spider's side of `xtask compare admission`
//! (#2318): no crawl, only Spider's admission of each link its crawl loop
//! finds, as `Website::crawl_raw`'s loop admits one: `is_allowed`, then
//! `is_allowed_disk`, then `insert_link`. It admits `--urls` new synthetic
//! links of one site, the ones Foxhound's side admits, then each again,
//! times each pass and writes the figures as one JSON object to `--out`. It
//! fails when a link of the first pass is refused or one of the second is
//! admitted, so its figures are an exact visited set's.
//!
//! Built with the `disk` feature, Spider's default here, it keeps the
//! visited links past `LINKS_VISITED_MEMORY_LIMIT` (15,000 unless that
//! variable says otherwise) in SQLite, in a file of the temporary folder;
//! built without it, Spider keeps every one in memory.
//!
//!   goodfetch-compare-spider --scenario admission --urls 1000000 \
//!     --out admission.json

use std::path::PathBuf;
use std::time::{Duration, Instant};

use spider::CaseInsensitiveString;
use spider::website::{ProcessLinkStatus, Website};

/// The site every link is found on.
const SEED: &str = "https://shop.test/";

/// The scenario's arguments.
struct Arguments {
    urls: usize,
    out: PathBuf,
}

/// Whether the command line asks for the admission scenario: its first
/// argument names it.
pub(crate) fn asked() -> bool {
    let mut given = std::env::args().skip(1);
    given.next().as_deref() == Some("--scenario") && given.next().as_deref() == Some("admission")
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: goodfetch-compare-spider --scenario admission --urls <n> --out <file>";
    let mut urls = None;
    let mut out = None;
    let mut given = std::env::args().skip(3);
    while let Some(name) = given.next() {
        let value = given.next().ok_or(usage)?;
        match name.as_str() {
            "--urls" => urls = Some(value.parse().map_err(|_| "--urls takes a count")?),
            "--out" => out = Some(PathBuf::from(value)),
            _ => return Err(usage.to_owned()),
        }
    }
    Ok(Arguments {
        urls: urls.ok_or(usage)?,
        out: out.ok_or(usage)?,
    })
}

/// The `n`th synthetic link, as Foxhound's side spells it.
fn synthetic(n: usize) -> CaseInsensitiveString {
    CaseInsensitiveString::from(format!("https://shop.test/aisle-{}/item-{n}", n % 64))
}

/// Whether Spider's crawl loop admits `link`, and if so notes it visited.
async fn admit(site: &mut Website, link: &CaseInsensitiveString) -> bool {
    let allowed = site.is_allowed(link);
    if allowed != ProcessLinkStatus::Allowed || !site.is_allowed_disk(link).await {
        return false;
    }
    site.insert_link(link).await;
    true
}

/// Admits every link once more and fails on the first whose answer is not
/// `admitted`. Each link is spelled inside the time, as Foxhound's side
/// spells each inside its own, so no list of them weighs on the memory
/// either side reaches.
async fn pass(site: &mut Website, urls: usize, admitted: bool) -> Result<Duration, String> {
    let started = Instant::now();
    for n in 0..urls {
        if admit(site, &synthetic(n)).await != admitted {
            let word = if admitted { "admitted" } else { "refused" };
            return Err(format!("link {n} was not {word}"));
        }
    }
    Ok(started.elapsed())
}

/// Nanoseconds per URL of `elapsed` over `urls`: both stay far below
/// 2^52, so no precision is lost.
fn per_url(elapsed: Duration, urls: usize) -> f64 {
    elapsed.as_nanos() as f64 / urls.max(1) as f64
}

async fn measure(arguments: &Arguments) -> Result<serde_json::Value, String> {
    let mut site = Website::new(SEED);
    // What `crawl_raw` sets up before its loop: the SQLite file, with the
    // `disk` feature.
    site.setup_disk();
    let seed = CaseInsensitiveString::from(SEED);
    if !admit(&mut site, &seed).await {
        return Err("the seed was not admitted".to_owned());
    }
    let added = pass(&mut site, arguments.urls, true).await?;
    let duplicates = pass(&mut site, arguments.urls, false).await?;
    Ok(serde_json::json!({
        "urls": arguments.urls,
        "disk": cfg!(feature = "disk"),
        "added_ns_per_url": per_url(added, arguments.urls),
        "duplicate_ns_per_url": per_url(duplicates, arguments.urls),
        "links_in_memory": site.size(),
        "added": arguments.urls,
        "duplicates": arguments.urls,
    }))
}

/// Runs the scenario; a failure is written to `--out` as `{"error": …}`
/// when the arguments name one.
pub(crate) async fn run() -> Result<(), String> {
    let arguments = arguments()?;
    let figures = match measure(&arguments).await {
        Ok(figures) => figures,
        Err(error) => serde_json::json!({ "error": error }),
    };
    std::fs::write(&arguments.out, figures.to_string())
        .map_err(|error| format!("writing {}: {error}", arguments.out.display()))?;
    match figures.get("error").and_then(serde_json::Value::as_str) {
        Some(error) => Err(error.to_owned()),
        None => Ok(()),
    }
}
