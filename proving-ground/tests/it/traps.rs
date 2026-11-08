//! The `traps` family (ADR 0025): items linked at several URLs of the same
//! page, two crawler traps (a calendar and an archive that page on without
//! end) and honeypot links hidden from people. The Proving ground catches a
//! crawl that fetches a page twice, stays in a trap or follows a honeypot
//! from its own record, and refuses the address that followed a honeypot.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::crawler::{Crawler, Fetched, every_url, field, finding_kinds, links};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 164;
const TASK: &str = "traps/1";

/// The known-right crawl: the fixed URL rules, no hidden link, and each trap
/// closed after a few of its pages.
fn known_right(proving_ground: &ProvingGround) -> Crawler<'_> {
    let mut crawler = Crawler::new(proving_ground);
    crawler.closes = vec![("/calendar/", 5), ("/archive/", 5)];
    crawler
}

fn records(pages: &[Fetched]) -> Vec<Value> {
    pages
        .iter()
        .filter_map(|page| {
            let body = &page.answer.body;
            Some(json!({
                "id": field(body, r#"<article class="item" data-id="([^"]+)""#)?,
                "name": field(body, r#"<h1 class="name">([^<]+)</h1>"#)?,
            }))
        })
        .collect()
}

fn start_url(proving_ground: &ProvingGround) -> String {
    proving_ground.task(TASK)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

#[test]
fn a_crawl_that_fetches_each_page_once_and_leaves_the_traps_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = known_right(&proving_ground).crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(answer["findings"], json!([]));
}

#[test]
fn a_crawl_missing_one_item_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = known_right(&proving_ground).crawl(&start_url(&proving_ground));
    let mut records = records(&pages);
    let dropped = records.remove(0);

    let answer = proving_ground.judge(&submission(TASK, "failure", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!([dropped["id"]]));
}

#[test]
fn a_crawl_that_fetches_every_url_of_a_page_is_caught_fetching_duplicates() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = known_right(&proving_ground);
    crawler.key = every_url;
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records_once(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(finding_kinds(&answer), ["duplicate"], "{answer}");
    assert_eq!(answer["missing"], json!([]));
}

#[test]
fn a_crawl_that_stays_in_a_trap_is_caught() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.limit = 200;
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(finding_kinds(&answer), ["trap"], "{answer}");
    assert_eq!(answer["missing"], json!([]));
}

#[test]
fn a_complete_crawl_that_fetched_duplicates_left_to_review_is_failed() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = known_right(&proving_ground);
    crawler.key = every_url;
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "needs-review", &records_once(&pages)));

    assert_eq!(answer["outcome"], "failed", "{answer}");
}

#[test]
fn a_crawl_that_follows_a_hidden_link_is_caught_and_its_address_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = known_right(&proving_ground);
    crawler.follow_hidden = true;
    let pages = crawler.crawl(&start_url(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert!(
        finding_kinds(&answer).contains(&"honeypot".to_owned()),
        "{answer}"
    );
    assert!(
        !answer["missing"].as_array().unwrap().is_empty(),
        "{answer}"
    );
}

#[test]
fn every_hidden_link_is_a_honeypot_that_flags_the_address_following_it() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);
    let home = proving_ground.get(&start).body;
    let visible = links(&home, false);
    let hidden: Vec<String> = links(&home, true)
        .into_iter()
        .filter(|link| !visible.contains(link))
        .collect();

    let before = proving_ground.get(&start).status;
    let honeypot = proving_ground.get(&format!("{}{}", start.trim_end_matches('/'), hidden[0]));
    let after = proving_ground.get(&start);

    assert_eq!(hidden.len(), 4, "{home}");
    assert!(
        hidden.iter().all(|link| link.starts_with("/hp/")),
        "{hidden:?}"
    );
    assert_eq!((before, honeypot.status, after.status), (200, 200, 403));
    assert!(after.body.contains("flagged"), "{}", after.body);
}

#[test]
fn robots_txt_disallows_the_honeypots() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);

    let robots = proving_ground.get(&format!("{start}robots.txt"));

    assert_eq!(robots.status, 200);
    assert_eq!(robots.body, "User-agent: *\nDisallow: /hp/\n");
}

/// The Records of a crawl, each item once, as a tool that fetched duplicates
/// but merged their Records returns them.
fn records_once(pages: &[Fetched]) -> Vec<Value> {
    let mut records = records(pages);
    records.sort_by_key(|record| record["id"].to_string());
    records.dedup();
    records
}

/// `/hidden-links`, which no page links, holds a honeypot under `/hp/` for
/// each way a link hides from people, eleven, beside the links a person
/// sees, reaches by scrolling or reveals, to items 1 to 13 in order.
#[test]
fn the_hidden_links_page_hides_eleven_honeypots_beside_thirteen_items() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);

    let page = proving_ground.get(&format!("{start}hidden-links"));
    let every = links(&page.body, true);

    assert_eq!(page.status, 200);
    assert_eq!(
        every.iter().filter(|link| link.starts_with("/hp/")).count(),
        11,
        "{}",
        page.body
    );
    assert_eq!(
        every
            .iter()
            .filter(|link| !link.starts_with("/hp/"))
            .cloned()
            .collect::<Vec<_>>(),
        (1..=13)
            .map(|number| format!("/items/I-{number:04}"))
            .collect::<Vec<_>>()
    );
    assert!(!links(&proving_ground.get(&start).body, true).contains(&"/hidden-links".to_owned()));
}

/// `/cards`, which no page links, links item 3 only through an image link
/// hidden from assistive technology, category 1 from a `hidden` menu then
/// from the visible one, and items 1 and 2 each through the hidden image
/// link and the visible title, item 1's image first and item 2's title
/// first.
#[test]
fn the_cards_page_hides_some_links_to_pages_a_person_also_reaches() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground);

    let page = proving_ground.get(&format!("{start}cards"));

    assert_eq!(page.status, 200);
    assert_eq!(
        links(&page.body, true),
        [
            "/items/I-0003",
            "/categories/1",
            "/items/I-0001",
            "/items/I-0001",
            "/items/I-0002",
            "/items/I-0002",
            "/categories/1",
        ],
        "{}",
        page.body
    );
    assert_eq!(page.body.matches("aria-hidden=\"true\"").count(), 3);
    assert!(page.body.contains("<nav hidden><a href=\"/categories/1\">"));
    assert!(!links(&proving_ground.get(&start).body, true).contains(&"/cards".to_owned()));
}
