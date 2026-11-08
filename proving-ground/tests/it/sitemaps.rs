//! The `sitemaps` family: a sitemap index of more sitemaps than the
//! protocol's 50,000, served plain and gzip-compressed, each generated on
//! request so no committed file is larger than 256 KiB. Its sitemaps are
//! empty but every 1,024th and those past its 50,000th entry, which list
//! one item each, so a crawl that truncates the index at the protocol's
//! limit loses items, and the oracle names them.
//! Covers: story:2-33

use std::io::Read as _;
use std::path::Path;

use proptest::prelude::*;
use regex::Regex;
use serde_json::{Value, json};

use crate::crawler::field;
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 1538;
/// The protocol's limit of entries in one sitemap index (sitemaps.org).
const PROTOCOL_LIMIT: usize = 50_000;
/// ADR 0062's limit of a committed file.
const COMMITTED_LIMIT: u64 = 256 * 1024;

fn start_url(proving_ground: &ProvingGround, task: &str) -> String {
    proving_ground.task(task)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

/// A sitemap's or a sitemap index's text, decompressed when it is gzip.
fn sitemap_text(proving_ground: &ProvingGround, url: &str) -> String {
    let answer = proving_ground.get_raw(url);
    assert_eq!(answer.status, 200, "{url}");
    if Path::new(url)
        .extension()
        .is_some_and(|extension| extension == "gz")
    {
        assert_eq!(answer.content_type.as_deref(), Some("application/gzip"));
        let mut text = String::new();
        flate2::read::GzDecoder::new(answer.body.as_slice())
            .read_to_string(&mut text)
            .expect("the body is gzip of UTF-8");
        text
    } else {
        String::from_utf8(answer.body).expect("the body is UTF-8")
    }
}

fn locs(text: &str) -> Vec<String> {
    Regex::new(r"<loc>([^<]*)</loc>")
        .expect("a valid pattern")
        .captures_iter(text)
        .map(|loc| loc[1].to_owned())
        .collect()
}

/// The number of a sitemap's URL, `…/sitemaps/<n>.xml`.
fn sitemap_number(url: &str) -> u64 {
    url.rsplit('/')
        .next()
        .and_then(|file| file.strip_suffix(".xml"))
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("not a numbered sitemap: {url}"))
}

/// Whether sitemap `number` lists an item, as the family states it; the
/// properties below hold the site to it, so a crawl may skip the others,
/// which list nothing, instead of fetching 50,000 empty sitemaps.
fn lists_an_item(number: u64) -> bool {
    number.is_multiple_of(1_024) || number > 50_000
}

/// A sitemap that lists an item: a multiple of 1,024 within the protocol's
/// limit, or one past it.
fn listing_sitemap() -> impl Strategy<Value = u64> {
    prop_oneof![(1..=48_u64).prop_map(|n| n * 1_024), 50_001..=50_048_u64]
}

/// The Records of a crawl of the sitemap index at `start`, each with the
/// place in the index of the sitemap that listed it, from 0.
fn crawl(proving_ground: &ProvingGround, start: &str) -> Vec<(usize, Value)> {
    locs(&sitemap_text(proving_ground, start))
        .iter()
        .enumerate()
        .filter(|(_, sitemap)| lists_an_item(sitemap_number(sitemap)))
        .flat_map(|(at, sitemap)| {
            locs(&sitemap_text(proving_ground, sitemap))
                .into_iter()
                .map(move |item| (at, item))
        })
        .map(|(at, item)| {
            let body = proving_ground.get(&item).body;
            (
                at,
                json!({
                    "id": field(&body, r#"<article class="entry" data-id="([^"]+)""#),
                    "name": field(&body, r#"<h1 class="name">([^<]+)</h1>"#),
                }),
            )
        })
        .collect()
}

/// The Records of `crawled` whose sitemap is among the index's first
/// `entries`: those of a crawl that read no more of it.
fn within(crawled: &[(usize, Value)], entries: usize) -> Vec<Value> {
    crawled
        .iter()
        .filter(|(at, _)| *at < entries)
        .map(|(_, record)| record.clone())
        .collect()
}

#[test]
fn the_index_lists_more_sitemaps_than_the_protocol_allows_and_its_gzip_form_holds_the_same() {
    let proving_ground = ProvingGround::start(SEED);
    let gzip = start_url(&proving_ground, "sitemaps/2");
    let plain = gzip.strip_suffix(".gz").expect("a .gz index").to_owned();

    let text = sitemap_text(&proving_ground, &plain);

    assert_eq!(locs(&text).len(), 50_048);
    assert!(text.len() as u64 > COMMITTED_LIMIT, "{}", text.len());
    assert!(text.starts_with("<?xml"), "{}", &text[..80]);
    assert!(text.contains("<sitemapindex"));
    assert_eq!(sitemap_text(&proving_ground, &gzip), text);
}

#[test]
fn no_committed_file_of_the_proving_ground_passes_256_kib() {
    fn largest(directory: &Path) -> (u64, String) {
        std::fs::read_dir(directory)
            .expect("a readable directory")
            .map(|entry| {
                let entry = entry.expect("a readable entry");
                let path = entry.path();
                if entry.file_type().expect("a readable type").is_dir() {
                    largest(&path)
                } else {
                    (
                        entry.metadata().expect("readable metadata").len(),
                        path.display().to_string(),
                    )
                }
            })
            .max()
            .unwrap_or_default()
    }

    let (size, path) = largest(Path::new(env!("CARGO_MANIFEST_DIR")));

    assert!(size <= COMMITTED_LIMIT, "{path} is {size} bytes");
}

/// Checks `property` of 64 sitemaps `numbers` draws, on one Proving ground.
fn for_sitemaps(
    numbers: impl Strategy<Value = u64>,
    property: impl Fn(&ProvingGround, u64, String) -> Result<(), TestCaseError>,
) {
    let proving_ground = ProvingGround::start(SEED);
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(64));
    let checked = runner.run(&numbers, |number| {
        let url = proving_ground.url("1.sitemaps.test", &format!("/sitemaps/{number}.xml"));
        property(&proving_ground, number, sitemap_text(&proving_ground, &url))
    });
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}

#[test]
fn a_listing_sitemap_lists_its_own_item() {
    for_sitemaps(listing_sitemap(), |proving_ground, number, text| {
        prop_assert_eq!(
            locs(&text),
            vec![proving_ground.url("1.sitemaps.test", &format!("/items/{number}"))]
        );
        Ok(())
    });
}

#[test]
fn every_other_sitemap_is_empty() {
    let other = (1..=50_000_u64).prop_filter("a multiple of 1,024", |n| !n.is_multiple_of(1_024));
    for_sitemaps(other, |_, _, text| {
        prop_assert!(text.contains("<urlset"), "{}", text);
        prop_assert_eq!(locs(&text), Vec::<String>::new());
        Ok(())
    });
}

#[test]
fn a_crawl_of_every_entry_is_verified_and_one_that_stops_at_the_protocols_limit_is_a_false_success()
{
    let proving_ground = ProvingGround::start(SEED);
    // The plain and gzip indexes list the same entries (the first test), so
    // one crawl of the gzip index gives both tasks' Records.
    let start = start_url(&proving_ground, "sitemaps/2");
    let crawled = crawl(&proving_ground, &start);
    let every = within(&crawled, usize::MAX);
    let truncated = within(&crawled, PROTOCOL_LIMIT);

    let verified = ["sitemaps/1", "sitemaps/2"]
        .map(|task| proving_ground.judge(&submission(task, "success", &every)));
    let answer = proving_ground.judge(&submission("sitemaps/2", "success", &truncated));

    assert_eq!(every.len(), 96);
    for verified in &verified {
        assert_eq!(verified["outcome"], "verified", "{verified}");
    }
    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let past_the_limit: Vec<String> = locs(&sitemap_text(&proving_ground, &start))
        [PROTOCOL_LIMIT..]
        .iter()
        .map(|sitemap| format!("M-{:05}", sitemap_number(sitemap)))
        .collect();
    assert_eq!(past_the_limit.len(), 48);
    assert_eq!(answer["missing"], json!(past_the_limit), "{answer}");
}
