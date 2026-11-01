//! The Extraction families' pages and tasks (spec #152, Determinism): each
//! page is generated on request from the seed, so the same seed serves the
//! same bytes and another seed other ones, and each family lists the same
//! tasks for every seed.
//! Covers: story:2-33, story:2-90

use proptest::prelude::*;
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;

/// A page whose content the seed draws, as a host name and a path.
fn seeded_page() -> impl Strategy<Value = (&'static str, String)> {
    prop_oneof![
        (1..=12_u32).prop_map(|n| ("extraction.test", format!("/products/X-{n:04}"))),
        (1..=2_u32).prop_map(|n| ("extraction.test", format!("/api/products/{n}"))),
        (25..=36_u32).prop_map(|n| ("extraction.test", format!("/churn/X-{n:04}"))),
        (37..=48_u32).prop_map(|n| ("extraction.test", format!("/variants/X-{n:04}"))),
        prop::sample::select(vec!["us", "de", "fr", "jp", "in"])
            .prop_map(|region| ("extraction.test", format!("/{region}/products/X-0100"))),
        prop::sample::select(vec![
            "shift-jis",
            "euc-kr",
            "gb18030",
            "big5",
            "windows-1251",
            "iso-8859-2",
            "iso-8859-15",
            "utf-16-bom",
            "contradicting-meta",
            "undeclared-utf-8",
            "undeclared-legacy",
            "latin1-label",
            "ascii-label",
            "meta-utf-16-label",
            "x-user-defined",
            "late-meta-in-head",
            "meta-after-body",
            "undecodable-byte",
        ])
        .prop_map(|id| ("encodings.test", format!("/pages/{id}"))),
        prop::sample::select(vec![
            "arabic",
            "hebrew",
            "japanese",
            "chinese",
            "hindi",
            "thai",
            "combining-marks",
            "emoji",
            "astral",
        ])
        .prop_map(|id| ("languages.test", format!("/pages/{id}"))),
        Just((
            "languages.test",
            "/w%C3%B6rter/%E6%97%A5%E6%9C%AC%E8%AA%9E?q=caf%C3%A9&ort=Z%C3%BCrich".to_owned()
        )),
        Just(("xn--bcher-kva.languages.test", "/".to_owned())),
        prop::sample::select(vec![
            "report.pdf",
            "archive.zip",
            "data.json",
            "untyped-page",
            "untyped-document",
            "download",
            "export.csv",
        ])
        .prop_map(|file| ("files.test", format!("/files/{file}"))),
    ]
}

fn page(proving_ground: &ProvingGround, (host, path): &(&str, String)) -> Vec<u8> {
    let answer = proving_ground.get_raw(&format!("http://{host}:{}{path}", proving_ground.port()));
    assert!(
        answer.status == 200,
        "{host}{path}: {}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.body
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn the_same_seed_serves_byte_identical_pages(seed: u64, place in seeded_page()) {
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(seed);

        let pages = (page(&first, &place), page(&second, &place));

        prop_assert_eq!(pages.0, pages.1);
    }

    #[test]
    fn another_seed_serves_other_pages(seed: u64, other: u64, place in seeded_page()) {
        prop_assume!(seed != other);
        let first = ProvingGround::start(seed);
        let second = ProvingGround::start(other);

        let pages = (page(&first, &place), page(&second, &place));

        prop_assert_ne!(pages.0, pages.1);
    }
}

/// Each task of the Extraction families as its ID, kind and host name.
fn listed(seed: u64) -> Vec<Value> {
    let proving_ground = ProvingGround::start(seed);
    let port = proving_ground.port().to_owned();
    proving_ground.tasks()["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .filter(|task| {
            ["extraction", "encodings", "languages", "files"]
                .contains(&task["family"].as_str().expect("a family"))
        })
        .map(|task| {
            let start_url = task["start_url"].as_str().expect("a start URL");
            let host = start_url
                .strip_prefix("http://")
                .and_then(|rest| rest.split_once(&format!(":{port}/")))
                .expect("the start URL is on a host name and the bound port")
                .0;
            json!([task["id"], task["kind"], host])
        })
        .collect()
}

#[test]
fn each_family_lists_the_same_tasks_on_their_own_host_names_for_every_seed() {
    let tasks = (listed(165), listed(2));

    assert_eq!(tasks.0, tasks.1);
    assert_eq!(
        tasks.0,
        [
            json!(["extraction/1", "crawl", "1.extraction.test"]),
            json!(["extraction/2", "extract", "2.extraction.test"]),
            json!(["extraction/3", "crawl", "3.extraction.test"]),
            json!(["extraction/4", "crawl", "4.extraction.test"]),
            json!(["extraction/5", "extract", "5.extraction.test"]),
            json!(["extraction/6", "crawl", "6.extraction.test"]),
            json!(["extraction/7", "crawl", "7.extraction.test"]),
            json!(["extraction/8", "crawl", "8.extraction.test"]),
            json!(["extraction/9", "crawl", "9.extraction.test"]),
            json!(["extraction/10", "crawl", "10.extraction.test"]),
            json!(["encodings/1", "crawl", "1.encodings.test"]),
            json!(["encodings/2", "crawl", "2.encodings.test"]),
            json!(["languages/1", "crawl", "1.languages.test"]),
            json!(["languages/2", "extract", "xn--bcher-kva.languages.test"]),
            json!(["files/1", "crawl", "1.files.test"]),
            json!(["files/2", "extract", "2.files.test"]),
            json!(["files/3", "extract", "3.files.test"]),
        ]
    );
}

#[test]
fn the_huge_page_is_the_same_for_the_same_seed() {
    let first = ProvingGround::start(165);
    let second = ProvingGround::start(165);
    let place = ("extraction.test", "/huge".to_owned());

    let pages = (page(&first, &place), page(&second, &place));

    assert!(pages.0 == pages.1, "the huge pages differ");
}
