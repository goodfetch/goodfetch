//! Foxhound's handled crawl through its Rust SDK, for `just compare`'s
//! handled scenario (#2858): the TypeScript row's program
//! (`xtask/compare-foxhound/crawl.ts`) written in Rust, its engine linked
//! into this process, as the Rust SDK runs one. From `--url`, under the
//! Label `page`, each page's handler reads its twenty fields in one
//! `page.read` (ADR 0147), pushes them as one Record into the Dataset
//! `--dataset` and enqueues the links of its nav, its children's list and
//! its home link, under `page` again: four exchanges a page with the
//! handler call. `--settings` are the settings xtask gives the TypeScript
//! program's Run, with no `fetch.tier`, so the engine chooses each page's
//! tier (#1602); the Rust SDK's crawl start takes no settings of its own,
//! so they are the connection's, which every Run it starts inherits. It
//! prints one line, `pages=<n>`, the Run's requests that succeeded, each a
//! handler call that ended, once the Run has finished complete; xtask
//! reads the Records from the Run's file afterwards. `just compare` builds
//! it with the cell's profile.
//!
//! ```text
//! compare-handled --url http://scale.test/sites/1000/1 --settings '{…}' \
//!   --run compare --dataset pages
//! ```
#![expect(
    clippy::print_stdout,
    reason = "`just compare`'s handled program (ADR 0025): its count, the one line xtask reads"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_foxhound::settings::{
    Settings, crawl, dataset, egress, fetch, frontier, network, politeness, provenance, run_state,
    site_preferences,
};
use goodfetch_foxhound::{
    BoxError, ConnectOptions, Crawl, CrawlContext, EngineChoice, FieldLocator, FieldTransform,
    Foxhound, PageField, RunId, Seed, ValueField,
};
use goodfetch_protocol::run::{Completeness, Lifecycle};
use serde_json::{Map, Value};

/// The program's arguments, each given once.
struct Arguments {
    url: String,
    settings: Settings,
    run: RunId,
    dataset: String,
}

/// How the program is run.
const USAGE: &str =
    "usage: compare-handled --url <url> --settings <json> --run <run id> --dataset <name>";

/// The arguments from the command line, or why they do not do.
fn arguments() -> Result<Arguments, BoxError> {
    let mut given: BTreeMap<String, String> = BTreeMap::new();
    let mut words = std::env::args().skip(1);
    while let Some(flag) = words.next() {
        let name = match flag.as_str() {
            "--url" | "--settings" | "--run" | "--dataset" => flag,
            _ => return Err(format!("unknown argument `{flag}`; {USAGE}").into()),
        };
        let value = words
            .next()
            .ok_or_else(|| format!("{name} takes a value; {USAGE}"))?;
        if given.insert(name.clone(), value).is_some() {
            return Err(format!("{name} is given twice; {USAGE}").into());
        }
    }
    let mut take = |name: &str| {
        given
            .remove(name)
            .ok_or_else(|| BoxError::from(format!("{name} is missing; {USAGE}")))
    };
    let url = take("--url")?;
    let settings = take("--settings")?;
    let run = take("--run")?;
    let dataset = take("--dataset")?;
    let Value::Object(settings) = serde_json::from_str(&settings)? else {
        return Err("--settings takes a JSON object of the Run's settings".into());
    };
    Ok(Arguments {
        url,
        settings: typed(&settings)?,
        run: RunId::parse(&run)?,
        dataset,
    })
}

/// The settings `json` holds, nested by their keys' segments, as the Rust
/// SDK's typed keys: each key the handled scenario gives, and an error for
/// any other, so none is dropped unread.
fn typed(json: &Map<String, Value>) -> Result<Settings, BoxError> {
    let mut leaves = Vec::new();
    flatten("", json, &mut leaves);
    let mut settings = Settings::new();
    for (key, value) in leaves {
        let flag = || serde_json::from_value::<bool>(value.clone());
        let count = || serde_json::from_value::<u64>(value.clone());
        settings = match key.as_str() {
            "network.resolve" => settings.set(
                network::RESOLVE,
                serde_json::from_value::<BTreeMap<String, Vec<String>>>(value)?,
            ),
            "egress.private_ranges" => settings.set(
                egress::PRIVATE_RANGES,
                serde_json::from_value::<Vec<String>>(value)?,
            ),
            "fetch.block_classification" => settings.set(fetch::BLOCK_CLASSIFICATION, flag()?),
            "frontier.learning" => settings.set(frontier::LEARNING, flag()?),
            "provenance.log" => settings.set(provenance::LOG, flag()?),
            "dataset.records" => settings.set(dataset::RECORDS, flag()?),
            "site_preferences.respect_robots_txt" => {
                settings.set(site_preferences::RESPECT_ROBOTS_TXT, flag()?)
            }
            "politeness.domain.delay" => {
                settings.set(politeness::domain::DELAY, Duration::from_millis(count()?))
            }
            "politeness.domain.concurrency" => {
                settings.set(politeness::domain::CONCURRENCY, count()?)
            }
            "crawl.handler.concurrency" => settings.set(crawl::handler::CONCURRENCY, count()?),
            "run_state.durability" => {
                let durability = match value.as_str() {
                    Some("full") => run_state::Durability::Full,
                    Some("fast") => run_state::Durability::Fast,
                    Some("batch") => run_state::Durability::Batch,
                    Some("off") => run_state::Durability::Off,
                    _ => return Err(format!("run_state.durability takes no {value}").into()),
                };
                settings.set(run_state::DURABILITY, durability)
            }
            _ => return Err(format!("the program sets no `{key}`").into()),
        };
    }
    Ok(settings)
}

/// Each leaf of `json` under `prefix`, by its dotted key: `network.resolve`
/// is one, its object a map of hosts.
fn flatten(prefix: &str, json: &Map<String, Value>, leaves: &mut Vec<(String, Value)>) {
    for (name, value) in json {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match value {
            Value::Object(inner) if key != "network.resolve" => flatten(&key, inner, leaves),
            _ => leaves.push((key, value.clone())),
        }
    }
}

/// The element `selector` matches, read as its text, or its `attribute`.
fn css(selector: &str, attribute: Option<&str>) -> PageField {
    PageField::Value(ValueField {
        locator: FieldLocator::Css {
            selector: selector.to_owned(),
        },
        fallbacks: Vec::new(),
        attribute: attribute.map(str::to_owned),
        matched: None,
        transform: FieldTransform::Text,
    })
}

const ITEM: &str = "article.item";

/// A child's link in the page's list, the `k`th.
fn child(k: u32) -> String {
    format!("ul.pages > li:nth-child({k}) > a")
}

/// A page of the article site's twenty fields.
fn fields() -> BTreeMap<String, PageField> {
    let mut fields: BTreeMap<String, PageField> = [
        ("title", css("title", None)),
        ("name", css(&format!("{ITEM} > h1.name"), None)),
        ("id", css(ITEM, Some("data-id"))),
        ("home", css("nav > a", Some("href"))),
        ("home_label", css("nav > a", None)),
        ("header", css("header > p", None)),
        ("nav", css("nav > p", None)),
        ("footer", css("footer > p", None)),
        ("first_child", css(&child(1), Some("href"))),
        ("first_child_name", css(&child(1), None)),
        ("second_child", css(&child(2), Some("href"))),
        ("second_child_name", css(&child(2), None)),
    ]
    .into_iter()
    .map(|(name, field)| (name.to_owned(), field))
    .collect();
    for index in 1..=8 {
        fields.insert(
            format!("paragraph_{index}"),
            css(&format!("{ITEM} > p:nth-of-type({index})"), None),
        );
    }
    fields
}

/// Crawls from the arguments' URL until the Run ends, and answers its
/// requests that succeeded: each a handler call that ended.
async fn crawled(arguments: Arguments) -> Result<u64, BoxError> {
    let mut crawl = Crawl::new();
    let page = crawl.label::<()>("page")?;
    let pages = crawl.dataset::<Map<String, Value>>(arguments.dataset)?;
    let (fields, next) = (Arc::new(fields()), page.clone());
    crawl.handle(&page, move |ctx: CrawlContext<()>| async move {
        let page = ctx.page()?;
        let read = page.read(fields.as_ref().clone()).await?.into_inner();
        ctx.push(&pages, &read.values).await?;
        ctx.enqueue_links(&page.locator("ul.pages a, nav > a"), &next, ())
            .await?;
        Ok(())
    })?;

    let options = ConnectOptions::default()
        .engine(EngineChoice::Linked)
        .settings(arguments.settings);
    let foxhound = Foxhound::connect(options).await?;
    let ended = async {
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&arguments.url, &page, ())])
            .run_id(&arguments.run)
            .await?;
        let report = crawl.serve(&run).await?;
        let (ended, requests) = (&report.report, report.requests.as_ref());
        match requests {
            Some(requests)
                if ended.lifecycle == Lifecycle::Finished
                    && ended.completeness == Completeness::Complete =>
            {
                Ok(requests.succeeded)
            }
            _ => Err(BoxError::from(format!(
                "the crawl did not finish complete: {report:?}"
            ))),
        }
    }
    .await;
    foxhound.close().await?;
    ended
}

fn main() -> Result<(), BoxError> {
    let arguments = arguments()?;
    let runtime = goodfetch_runtime::multi_thread().enable_all().build()?;
    let handled = runtime.block_on(crawled(arguments))?;
    println!("pages={handled}");
    Ok(())
}
